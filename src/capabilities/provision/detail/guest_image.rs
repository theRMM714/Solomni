//! 目的：guest 根文件系统镜像的装配根——把 deb 选型、initrd 配方、ext4 属性三块拼成一条 install。
//! 管：`Ext4Images` 适配器、`install_guest_rootfs` 的多步构造、基线打包与系统清理。
//! 不管：deb / 内核 / agent 的选型与解包（`deb.rs`）、initrd 配方与脚本（`initrd.rs`）、ext4 属性与 debugfs 工具（`image.rs`）。
//! 联动：service 的 guest-rootfs 分支经 GuestImage 调进来；本目录是 provision 里唯一碰 ext4 工具链的地方。

mod deb;
mod image;
mod initrd;

pub(crate) use deb::*;
pub(crate) use image::*;
pub(crate) use initrd::*;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::capabilities::provision::detail::host::{ensure_executable, run_tool, symlink};
use crate::capabilities::provision::domain::{Dep, link, under};
use crate::capabilities::provision::ports::{Fetcher, GuestImage};

/// 目的：ext4 基线镜像的适配器——下载（内核 / QGA）经 Fetcher，落盘与工具链在本文件。
pub struct Ext4Images {
    root: PathBuf,
    fetch: Arc<dyn Fetcher>,
}

impl Ext4Images {
    pub fn new(root: &Path, fetch: Arc<dyn Fetcher>) -> Ext4Images {
        Ext4Images {
            root: root.to_path_buf(),
            fetch,
        }
    }
}

impl GuestImage for Ext4Images {
    fn install(
        &self,
        dep: &Dep,
        cached: &Path,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        install_guest_rootfs(&self.root, dep, cached, self.fetch.as_ref(), on_line)
    }
}

/// 目的：清掉 cloud 镜像留下的、在 9p 根下会掉进 emergency 的东西。
pub(crate) fn sanitize_guest_base(base: &Path) -> Result<(), String> {
    fs::write(
        base.join("etc").join("fstab"),
        "# guest root 由内核命令行给出（磁盘镜像 /dev/vda），这里不写 fstab 条目。
",
    )
    .map_err(|e| format!("写 fstab 失败：{}", e))?;
    let mask = base
        .join("etc")
        .join("systemd")
        .join("system")
        .join("systemd-remount-fs.service");
    if mask.symlink_metadata().is_err() {
        fs::create_dir_all(mask.parent().expect("有父目录")).map_err(|e| e.to_string())?;
        symlink("/dev/null", &mask).map_err(|e| format!("mask remount 失败：{}", e))?;
    }
    Ok(())
}

/// 目的：Linux guest 基础根的多步构造——cloud 镜像 → rootfs 目录 + 通用内核 + 9p initrd + qemu-guest-agent。
/// 约束：只写项目内；调系统工具（sfdisk / dd / debugfs / ar / unshare），不依赖任何 Node 脚本；源在 links.yaml 的 guest.*，镜像 tar 已由调用方下到缓存。
pub(crate) fn install_guest_rootfs(
    root: &Path,
    dep: &Dep,
    image_tar: &Path,
    fetch: &dyn Fetcher,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let family = family_of(dep)?;
    let base = under(root, &dep.install);
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&base).map_err(|e| format!("建 {} 失败：{}", base.display(), e))?;
    let disk = cache.join("disk.raw");
    if !disk.is_file() {
        on_line("解开 cloud 镜像 tar（disk.raw）");
        let x = cache.join("image-x");
        let _ = fs::remove_dir_all(&x);
        fs::create_dir_all(&x).map_err(|e| format!("建 {} 失败：{}", x.display(), e))?;
        // 2 GB 的 disk.raw 在 tar 里是 **GNU sparse**：用系统 tar 展开（纯 Rust 解包暂不支持稀疏项）。
        run_tool(
            std::process::Command::new("tar")
                .arg("-xf")
                .arg(image_tar)
                .arg("-C")
                .arg(&x),
            "tar",
        )?;
        let found = fs::read_dir(&x)
            .map_err(|e| format!("读 {} 失败：{}", x.display(), e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.file_name().map(|n| n == "disk.raw").unwrap_or(false))
            .ok_or_else(|| "cloud 镜像 tar 里没有 disk.raw".to_string())?;
        fs::rename(&found, &disk).map_err(|e| format!("搬 disk.raw 失败：{}", e))?;
        let _ = fs::remove_dir_all(&x);
    } else {
        on_line("用已有 disk.raw（缓存）");
    }
    let rootfs = cache.join("rootfs.ext4");
    if !rootfs.is_file() {
        on_line("从 GPT 取 root 分区（sfdisk + dd）");
        let (start, size) = root_partition(&disk)?;
        run_tool(
            std::process::Command::new("dd")
                .arg(format!("if={}", disk.display()))
                .arg(format!("of={}", rootfs.display()))
                .arg("bs=512")
                .arg(format!("skip={}", start))
                .arg(format!("count={}", size))
                .arg("status=none"),
            "dd",
        )?;
    }
    // 重新构造前先清掉上一次的镜像产物：留着它，rdump 出来的目录里就多一份旧 rootfs.ext4，
    // 打新镜像时会被整个包进去（体积翻倍、内容还是过期的）。
    let stale_image = base.join("rootfs.ext4");
    if stale_image.is_file() {
        fs::remove_file(&stale_image).map_err(|e| format!("清旧镜像失败：{}", e))?;
    }
    if !base.join("etc").join("os-release").is_file() {
        on_line(&format!("把 rootfs 解到 {}（debugfs rdump）", dep.install));
        extract_guest_rootfs(&rootfs, &base)?;
    }
    // 这时目录树与源镜像一一对应：先把"打包会丢的属性"读出来（之后会装内核 / agent，树不再对应）。
    let attrs = guest_attrs_to_restore(&rootfs, &base)?;
    let abi = ensure_guest_kernel(&cache, &base, fetch, on_line)?;
    ensure_guest_qga(&cache, &base, fetch, on_line)?;
    sanitize_guest_base(&base)?;
    ensure_guest_initrd(&base, &abi, family, on_line)?;
    link_guest_kernel(&base, &abi)?;
    let py3 = base.join("usr").join("bin").join("python3");
    let py = base.join("usr").join("bin").join("python");
    if py3.is_file() && !py.exists() {
        let _ = symlink("python3", &py);
    }
    // 画像：虚拟根对世界的自我介绍（列表显示名与 os 从这里读）；运行能力由运行期编排另写。
    let yaml = base.join("guest.yaml");
    if !yaml.is_file() {
        fs::write(
            &yaml,
            "# 由产品生成：命令通道 QGA；共享客户端与运行能力在运行期编排（guest-runtime）里确认后另写。\n\nid: debian-13-genericcloud-amd64\nos: linux\narch: amd64\nagent: qga\nclients: [9p, virtiofs]\nruntimes: []\n",
        )
        .map_err(|e| format!("写 guest.yaml 失败：{}", e))?;
        on_line("写 guest.yaml（QGA 通道画像）");
    }
    // 运行产物是**磁盘镜像**（会话靠每租约可写层并发，谁也不写基础镜像）；目录只在这条构造链里当中间产物。
    pack_guest_base(&base, &cache, on_line)?;
    // rdump 与 mke2fs 都拿不到源镜像里的特权位 / 属主、属组：按刚读出的清单在**镜像上**回填并核对。
    if !attrs.is_empty() {
        on_line(&format!(
            "按源镜像回填 {} 条属性（特权位 / 属主）",
            attrs.len()
        ));
    }
    restore_guest_image_attrs(&base.join("rootfs.ext4"), &attrs)?;
    // agent 已经装进镜像：运行期编排看到这个标记就跳过 cloud-init 首启。
    fs::write(
        base.join(".qga-ready"),
        "guest-rootfs 已装 qemu-guest-agent（打包进磁盘镜像）\n",
    )
    .map_err(|e| format!("写 .qga-ready 失败：{}", e))?;
    Ok(())
}

/// 目的：把构造好的 rootfs 目录打成**磁盘镜像**（`rootfs.ext4`，raw），然后删掉目录中间产物。
/// 参数：base = 虚拟根目录（内核 / initrd / guest.yaml 与镜像并排）；cache = 打包临时落点；on_line = 进度行。
/// 约束：只在 Linux 宿主上跑——`unshare -r` 拿假 root，镜像里的属主才会是 0（否则会写成调用者 uid）；
///   调用前目录已定稿（内核 / agent / initrd / 画像都写完）。
pub(crate) fn pack_guest_base(
    base: &Path,
    cache: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    // 内核 / initrd 现在是软链（`vmlinuz` → `boot/vmlinuz-<abi>`）：落成实体文件，否则删目录后就成了悬空链。
    for name in ["vmlinuz", "initrd.img"] {
        let link = base.join(name);
        if fs::symlink_metadata(&link).is_err()
            || !fs::symlink_metadata(&link)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            continue;
        }
        let tmp = base.join(format!("{}.solid", name));
        fs::copy(&link, &tmp).map_err(|e| format!("把 {name} 落成实体文件失败：{e}"))?;
        fs::remove_file(&link).map_err(|e| format!("换掉 {name} 软链失败：{e}"))?;
        fs::rename(&tmp, &link).map_err(|e| format!("放回 {name} 失败：{e}"))?;
    }
    let used = dir_bytes(base)?;
    // 留足余量：guest 里 apt 要下索引（压缩的也有几十 MB）、装 C/C++（约 300 MB）、解 Node（约 200 MB）、写日志——
    // 1.6 GiB 的旧镜像在维护启动装基础工具链时会真的空间不足（Write error 28）。下限 1 GB、至少 1.5 GB 余量。
    let slack = std::cmp::max(1536 << 20, used / 2);
    let need = std::cmp::max(1u64 << 30, used + slack);
    let blocks = need.div_ceil(4096);
    let tmp_img = cache.join("pack.ext4");
    let _ = fs::remove_file(&tmp_img);
    on_line(&format!(
        "把 rootfs 打成磁盘镜像（{} MiB，目录占用 {} MiB）",
        need >> 20,
        used >> 20
    ));
    // `-d` 从目录灌内容；假 root 里跑，镜像属主才是 root。**必须显式 `-b 4096`**：位置参数按块大小解释，
    // 不给时 mke2fs 会按体积选 1024 字节块（并套用 small 的 4096 字节/inode），镜像缩成约四分之一而装不下。
    run_tool(
        std::process::Command::new("unshare")
            .arg("-r")
            .arg("mke2fs")
            .args(["-q", "-b", "4096", "-t", "ext4", "-F", "-d"])
            .arg(base)
            .arg(&tmp_img)
            .arg(blocks.to_string()),
        "unshare + mke2fs（要 e2fsprogs 且内核允许非特权用户命名空间）",
    )?;
    let image = base.join("rootfs.ext4");
    fs::rename(&tmp_img, &image).map_err(|e| format!("放 rootfs.ext4 失败：{e}"))?;
    // 中间产物删掉：虚拟根只剩 磁盘镜像 + 内核 + initrd + 画像（没有第二份真相）。
    purge_except(
        base,
        &["rootfs.ext4", "vmlinuz", "initrd.img", "guest.yaml"],
    )?;
    on_line("磁盘镜像就位，目录中间产物已清掉");
    Ok(())
}
