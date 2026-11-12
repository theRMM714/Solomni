//! 目的：guest 基础根里 deb 与内核 / agent 的选型、解包与落位。
//! 管：分区定位、版本比较、包名挑选、ar 解包、ABI 判定、通用内核与 qemu-guest-agent 的装入、vmlinuz / initrd.img 软链。
//! 不管：initrd 怎么生成（`initrd.rs`）、ext4 落盘与属性（`guest_image.rs` / `image.rs`）。
//! 联动：`install_guest_rootfs` 按 内核 → initrd → agent 的顺序调用本模块。

use super::*;
/// 目的：从 GPT 里取 Linux root 分区（type GUID 优先，退第一个）的 (start, size)，单位 512 字节扇区。
pub(crate) fn root_partition(disk: &Path) -> Result<(u64, u64), String> {
    let out = std::process::Command::new("sfdisk")
        .arg("-J")
        .arg(disk)
        .output()
        .map_err(|e| format!("跑不了 sfdisk（需要 util-linux）：{}", e))?;
    if !out.status.success() {
        return Err("sfdisk 读分区表失败".to_string());
    }
    let table: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("sfdisk 输出不是 JSON：{}", e))?;
    let parts = table
        .get("partitiontable")
        .and_then(|t| t.get("partitions"))
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let root = parts
        .iter()
        .find(|p| {
            p.get("type").and_then(|t| t.as_str()) == Some("4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709")
        })
        .or_else(|| parts.first())
        .ok_or_else(|| "cloud 镜像里没有分区".to_string())?;
    let start = root
        .get("start")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "分区缺 start".to_string())?;
    let size = root
        .get("size")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "分区缺 size".to_string())?;
    Ok((start, size))
}

/// 目的：把版本号里的数字段零填充，得到一个可字典序比较的键（数字感知排序）。
pub(crate) fn version_key(s: &str) -> String {
    let mut out = String::new();
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push_str(&format!("{:0>12}", num));
                num.clear();
            }
            out.push(c);
        }
    }
    if !num.is_empty() {
        out.push_str(&format!("{:0>12}", num));
    }
    out
}

/// 目的：从目录页里挑出候选 .deb 名（前缀 + 必须包含两段判据），按版本号取最新。
pub(crate) fn pick_deb(text: &str, prefix: &str, must_contain: &str) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for tok in text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_' | '.' | '~' | ':'))
    }) {
        if tok.starts_with(prefix)
            && tok.ends_with(".deb")
            && tok.contains(must_contain)
            && !names.iter().any(|n| n == tok)
        {
            names.push(tok.to_string());
        }
    }
    names.into_iter().max_by_key(|n| version_key(n))
}

/// 目的：把 .deb 的 data.tar.* 解进目标（先 ar 取成员，不跑 maintainer 脚本）。
pub(crate) fn extract_deb(deb: &Path, dest: &Path, cache: &Path) -> Result<(), String> {
    let tmp = cache.join("deb-x");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| format!("建 {} 失败：{}", tmp.display(), e))?;
    let st = std::process::Command::new("ar")
        .arg("x")
        .arg(deb)
        .current_dir(&tmp)
        .status()
        .map_err(|e| format!("跑不了 ar（需要 binutils）：{}", e))?;
    if !st.success() {
        return Err("ar 解 .deb 失败".to_string());
    }
    let data = fs::read_dir(&tmp)
        .map_err(|e| format!("读 {} 失败：{}", tmp.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with("data.tar"))
                .unwrap_or(false)
        })
        .ok_or_else(|| ".deb 里没有 data.tar.*".to_string())?;
    let r = crate::kernel::detail::unpack::extract(&data, dest);
    let _ = fs::remove_dir_all(&tmp);
    r
}

/// 目的：基础根里**能拿来起 9p 根**的已装内核 ABI——排除 `-rt-`（实时内核，不是我们要的）与 `-cloud-`（没编 9p）。
/// 约束：`boot/vmlinuz-*` 只是候选；还要它的模块树里真有 9pnet_virtio，才算数。
pub(crate) fn usable_guest_abi(base: &Path) -> Option<String> {
    let boot = base.join("boot");
    let mut abis: Vec<String> = fs::read_dir(&boot)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_prefix("vmlinuz-").map(|s| s.to_string())
        })
        .collect();
    abis.sort();
    abis.into_iter()
        .find(|abi| !abi.contains("-rt-") && !abi.contains("-cloud-") && abi_has_9p(base, abi))
}

/// 目的：装入 Debian 通用内核（带 9p 的那些模块），返回内核 ABI。
/// 约束：cloud 镜像自带的 cloud 内核把 `CONFIG_NET_9P` 关了（`# CONFIG_NET_9P is not set`）——它**起不了 9p 根**，
///   所以只有"已装的 ABI 确实带 9pnet_virtio 模块"才算数，否则换与 rootfs 同 Debian 大版本的通用内核（deb 自带模块）。
pub(crate) fn ensure_guest_kernel(
    cache: &Path,
    base: &Path,
    fetch: &dyn Fetcher,
    on_line: &mut dyn FnMut(&str),
) -> Result<String, String> {
    if let Some(abi) = usable_guest_abi(base) {
        return Ok(abi);
    }
    let url =
        link("guest.debian.linux").ok_or_else(|| "links.yaml 缺 guest.debian.linux".to_string())?;
    let listing = fetch.text(&url)?;
    let major = debian_major(base);
    let name = pick_kernel_deb(&listing, major.as_deref())
        .ok_or_else(|| "内核源里没找到 amd64 通用内核包".to_string())?;
    let deb = cache.join(&name);
    if !deb.is_file() {
        fetch.download(&format!("{}{}", url, name), &deb, on_line)?;
    }
    on_line(&format!("装入通用内核（带 9p 模块）：{}", name));
    extract_deb(&deb, base, cache)?;
    let abi = abi_of_deb(&name).ok_or_else(|| format!("认不出内核包的 ABI：{}", name))?;
    // 换内核要连两个稳定软链一起换（否则 vmlinuz 还指着没有 9p 的 cloud 内核）。
    link_guest_kernel(base, &abi)?;
    Ok(abi)
}

/// 目的：这个 ABI 在基础根里有没有 9p 客户端模块（cloud 内核没有——它是模块，且 cloud 内核压根没编）。
pub(crate) fn abi_has_9p(base: &Path, abi: &str) -> bool {
    let dir = base
        .join("lib")
        .join("modules")
        .join(abi)
        .join("kernel")
        .join("net")
        .join("9p");
    fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok()).any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("9pnet_virtio.ko")
            })
        })
        .unwrap_or(false)
}

/// 目的：基础根是 Debian 第几版（`/etc/debian_version` 的主版本号）；读不到给 None（那就别挑版本）。
pub(crate) fn debian_major(base: &Path) -> Option<String> {
    let text = fs::read_to_string(base.join("etc").join("debian_version")).ok()?;
    let digits: String = text
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// 目的：从内核包文件名取 ABI（`linux-image-<abi>-unsigned_<版本>_<arch>.deb`）。
pub(crate) fn abi_of_deb(name: &str) -> Option<String> {
    let rest = name.strip_prefix("linux-image-")?;
    let rest = rest.split('_').next()?;
    rest.strip_suffix("-unsigned").map(|s| s.to_string())
}
/// 目的：从内核源清单里挑一个**普通**通用内核包名——`-rt-`（实时）与 `-cloud-`（没编 9p）都不能要。
/// 参数：major = rootfs 的 Debian 大版本号（有就优先同代内核，避免拿 deb14 的内核配 deb13 的 rootfs）。
pub(crate) fn pick_kernel_deb(listing: &str, major: Option<&str>) -> Option<String> {
    let pick =
        |must: &str| pick_deb(listing, "linux-image-", must).filter(|n| n.ends_with("_amd64.deb"));
    let exact = major.map(|m| format!("+deb{}-amd64-unsigned_", m));
    exact.as_deref().and_then(pick).or_else(|| {
        pick("-amd64-unsigned_").filter(|n| !n.contains("-rt-") && !n.contains("-cloud-"))
    })
}
/// 目的：装入 qemu-guest-agent 并让它随 virtio-serial 端口出现而启动。
pub(crate) fn ensure_guest_qga(
    cache: &Path,
    base: &Path,
    fetch: &dyn Fetcher,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if !base.join("usr").join("sbin").join("qemu-ga").is_file() {
        let url = link("guest.debian.pool")
            .ok_or_else(|| "links.yaml 缺 guest.debian.pool".to_string())?;
        let listing = fetch.text(&url)?;
        let name = pick_deb(&listing, "qemu-guest-agent_", "amd64.deb")
            .ok_or_else(|| "qemu 源里没找到 guest agent 包".to_string())?;
        let deb = cache.join(&name);
        if !deb.is_file() {
            fetch.download(&format!("{}{}", url, name), &deb, on_line)?;
        }
        on_line(&format!("装入 qemu-guest-agent：{}", name));
        extract_deb(&deb, base, cache)?;
    }
    // 可执行位：解归档没还原权限位的安装靠它自愈——agent 起不来就是这里的锅。
    ensure_executable(&base.join("usr").join("sbin").join("qemu-ga"));
    let wants = base
        .join("etc")
        .join("systemd")
        .join("system")
        .join("multi-user.target.wants");
    fs::create_dir_all(&wants).map_err(|e| format!("建 {} 失败：{}", wants.display(), e))?;
    let unit = wants.join("qemu-guest-agent.service");
    // 按**链接本身**在不在判断：它的目标是绝对路径，`exists()` 会在宿主上找，永远找不到。
    if unit.symlink_metadata().is_err() {
        symlink("/usr/lib/systemd/system/qemu-guest-agent.service", &unit)
            .map_err(|e| format!("建 qemu-guest-agent.service 软链失败：{}", e))?;
    }
    Ok(())
}

/// 目的：vmlinuz / initrd.img 这两个稳定软链指向当前 ABI（指错了就换指——换内核时会用到）。
pub(crate) fn link_guest_kernel(base: &Path, abi: &str) -> Result<(), String> {
    for (link_name, target) in [
        ("vmlinuz", format!("boot/vmlinuz-{}", abi)),
        ("initrd.img", format!("boot/initrd.img-{}", abi)),
    ] {
        let p = base.join(link_name);
        if let Ok(cur) = fs::read_link(&p) {
            if cur == Path::new(&target) {
                continue;
            }
        }
        // 名字被占着就先清掉（软链 / 普通文件都算）；再无条件建——不看 `exists()`（它跟着目标走，会骗人）。
        if p.symlink_metadata().is_ok() {
            fs::remove_file(&p).map_err(|e| format!("换 {} 软链失败：{}", link_name, e))?;
        }
        symlink(&target, &p).map_err(|e| format!("建 {} 软链失败：{}", link_name, e))?;
    }
    Ok(())
}
