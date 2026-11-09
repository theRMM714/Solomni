//! 目的：guest initrd 的发行版配方与生成——chroot 脚本、模块合并、产物核对与失败文案。
//! 管：`GuestFamily` 配方表、initramfs-tools 配置与脚本、模块去重合并、unshare + chroot 执行、失败时的输出尾。
//! 不管：deb 选型与内核装入（`deb.rs`）、ext4 落盘（`mod.rs`）。
//! 联动：`deb.rs` 的 `ensure_guest_kernel` 与 `install_guest_rootfs` 调 `ensure_guest_initrd`。

use super::*;
/// 目的：一个 Linux 虚拟根发行版族的构造配方——initrd 的差异只在这里，执行器（unshare + chroot + 固定 PATH）共用。
/// 约束：内核 / QGA / 系统清理 / 运行期编排仍按 Debian 写死；第二族落地时按同一份清单字段扩展。
#[derive(Debug)]
pub(crate) struct GuestFamily {
    /// 目的：initrd 配置（虚拟根内相对路径）。
    pub(crate) initrd_conf: &'static str,
    /// 目的：把模块清单写成该族配置文件的全文。
    pub(crate) initrd_conf_text: fn(&[String]) -> String,
    /// 目的：在 chroot 里跑的构建脚本（已含固定 PATH）。
    pub(crate) initrd_script: fn(&str) -> String,
    /// 目的：initrd 产物（虚拟根内相对路径）。
    pub(crate) initrd_artifact: fn(&str) -> String,
}

/// 目的：initrd 要补的模块——9p 回退档要它们；模块名各族一致，只有写法由配方给。
pub(crate) const INITRD_MODULES: &[&str] = &[
    "9p",
    "9pnet",
    "9pnet_virtio",
    "virtio",
    "virtio_pci",
    "virtio_ring",
];

/// 目的：按清单的 family 取构造配方；当前只有 Debian。
pub(crate) fn guest_family(name: &str) -> Option<&'static GuestFamily> {
    match name {
        "debian" => Some(&DEBIAN),
        _ => None,
    }
}

/// 目的：这条 guest-rootfs 用哪份配方——缺声明与不认识都如实拒绝，并指出预构造镜像的出路。
pub(crate) fn family_of(dep: &Dep) -> Result<&'static GuestFamily, String> {
    match dep.family.as_deref() {
        Some(name) => guest_family(name).ok_or_else(|| {
            format!(
                "{}：没有发行版族 {} 的构造配方（当前只有 debian）；可用预构造镜像 + guest.yaml",
                dep.id, name
            )
        }),
        None => Err(format!(
            "{}：清单缺 family（这条 guest-rootfs 用哪份构造配方）；可用预构造镜像 + guest.yaml",
            dep.id
        )),
    }
}

/// 目的：Debian / Ubuntu 的 initrd 配方（initramfs-tools）——一行一个模块。
static DEBIAN: GuestFamily = GuestFamily {
    initrd_conf: "etc/initramfs-tools/modules",
    initrd_conf_text: initramfs_tools_conf,
    initrd_script: initramfs_tools_script,
    initrd_artifact: initramfs_tools_artifact,
};

/// 目的：initramfs-tools 的模块文件全文。
pub(crate) fn initramfs_tools_conf(modules: &[String]) -> String {
    let mut text = modules.join("\n");
    text.push('\n');
    text
}

/// 目的：initramfs-tools 的构建脚本——先固定 guest PATH，宿主 PATH 缺 /usr/sbin 也能找到 depmod / mkinitramfs。
pub(crate) fn initramfs_tools_script(abi: &str) -> String {
    format!(
        "#!/bin/bash\nset -e\nexport PATH=/usr/sbin:/usr/bin:/sbin:/bin\ndepmod {0}\nmkinitramfs -o /boot/initrd.img-{0} {0}\n",
        abi
    )
}

/// 目的：initramfs-tools 的产物（虚拟根内相对路径）。
pub(crate) fn initramfs_tools_artifact(abi: &str) -> String {
    format!("boot/initrd.img-{}", abi)
}

/// 目的：把已有的 initrd 模块清单与要补的模块合并——去空行、保留原顺序、不重复。
pub(crate) fn merge_initrd_modules(existing: &str, wanted: &[&str]) -> Vec<String> {
    let mut lines: Vec<String> = existing
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for m in wanted {
        if !lines.iter().any(|l| l == m) {
            lines.push((*m).to_string());
        }
    }
    lines
}

/// 目的：拼出 initrd 构建失败的事实——退出码 + 输出尾部；不替内核能力下结论。
pub(crate) fn initrd_failure(code: Option<i32>, stderr: &[u8], stdout: &[u8]) -> String {
    let tail = output_tail(stderr, stdout);
    let status = match code {
        Some(code) => format!("退出码 {}", code),
        None => "被信号终止".to_string(),
    };
    if tail.is_empty() {
        format!("生成 initrd 失败（{}，无输出）", status)
    } else {
        format!("生成 initrd 失败（{}）：{}", status, tail)
    }
}

/// 目的：取子进程输出的尾部（stderr 在前、stdout 在后，最多 8 行，每行最多 300 字）。
pub(crate) fn output_tail(stderr: &[u8], stdout: &[u8]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for bytes in [stderr, stdout] {
        for line in String::from_utf8_lossy(bytes).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            lines.push(line.chars().take(300).collect());
        }
    }
    let start = lines.len().saturating_sub(8);
    lines[start..].join("\n")
}

/// 目的：在 guest 里生成带 9p 模块的 initrd（chroot 走非特权用户命名空间，不需要 root）。
/// 参数：family = 这条虚拟根的发行版族配方（配置路径 / 模块写法 / 构建脚本 / 产物）。
/// 约束：脚本在 chroot 里固定 PATH；失败带退出码与输出尾部，不把非零退出归因到命名空间。
pub(crate) fn ensure_guest_initrd(
    base: &Path,
    abi: &str,
    family: &GuestFamily,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let conf = base.join(family.initrd_conf);
    fs::create_dir_all(conf.parent().expect("有父目录")).map_err(|e| e.to_string())?;
    let modules = merge_initrd_modules(
        &fs::read_to_string(&conf).unwrap_or_default(),
        INITRD_MODULES,
    );
    fs::write(&conf, (family.initrd_conf_text)(&modules))
        .map_err(|e| format!("写 {} 失败：{}", conf.display(), e))?;
    let script = base.join("tmp").join("solomni-initrd.sh");
    fs::create_dir_all(script.parent().expect("有父目录")).map_err(|e| e.to_string())?;
    fs::write(&script, (family.initrd_script)(abi))
        .map_err(|e| format!("写 initrd 脚本失败：{}", e))?;
    on_line("在 chroot（用户命名空间，无需 root）里生成带 9p 的 initrd");
    // /proc 绑定失败不致命（Debian 的 mkinitramfs 不需要它），但要如实留一行，不静默吞掉。
    let inner = format!(
        "if mount --bind /proc {proc} 2>/dev/null; then :; else echo 'solomni: /proc 绑定失败（继续）' >&2; fi; chroot {root} /bin/bash /tmp/solomni-initrd.sh",
        proc = base.join("proc").display(),
        root = base.display()
    );
    let out = std::process::Command::new("unshare")
        .args(["-r", "-m", "sh", "-c", &inner])
        .output()
        .map_err(|e| format!("起不了 unshare（需要 util-linux）：{}", e))?;
    let _ = fs::remove_file(&script);
    if !out.status.success() {
        return Err(initrd_failure(out.status.code(), &out.stderr, &out.stdout));
    }
    // 成功时把 chroot 的告警带出来（限 5 行，别刷屏）：/proc 绑定失败这类事实不能只在日志里。
    for line in String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
    {
        on_line(&format!("initrd：{}", line.trim()));
    }
    let artifact = base.join((family.initrd_artifact)(abi));
    if !artifact.is_file() {
        return Err(format!(
            "initrd 没生成出来：{}（chroot 脚本退出 0，但没有产物）",
            artifact.display()
        ));
    }
    Ok(())
}
