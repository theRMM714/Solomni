//! 目的：ext4 镜像的属性回填与调试工具——debugfs 封装、目录清单、属主 / 特权位恢复、目录占用与清理。
//! 管：`GuestAttr` 与回填、`debugfs` 命令拼装与引号、目录占用统计、保留清单之外的清理。
//! 不管：镜像怎么装出来（`mod.rs`）、内核与 initrd（`deb.rs` / `initrd.rs`）。
//! 联动：`install_guest_rootfs` 与 `pack_guest_base` 用本模块做属性回填与中间产物清理。

use super::*;
/// 目的：一条"打包会丢的属性"——镜像内绝对路径 + 完整 mode（含类型位）+ uid / gid。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestAttr {
    path: String,
    mode: u32,
    uid: u32,
    gid: u32,
}

/// 目的：把源 rootfs 分区解成目录树（debugfs rdump）。
/// 约束：普通用户身份解——内核会剥掉特权位、rdump 也改不动属主；这两样由
///   `guest_attrs_to_restore` / `restore_guest_image_attrs` 在打包后按源镜像回填。
pub(crate) fn extract_guest_rootfs(image: &Path, dest: &Path) -> Result<(), String> {
    run_tool(
        std::process::Command::new("debugfs")
            .arg("-R")
            .arg(format!("rdump / {}", dest.display()))
            .arg(image),
        "debugfs",
    )
}

/// 目的：源镜像里"打包会丢"的属性——特权位（setuid / setgid / sticky）、非 root 属主 / 属组。
/// 参数：tree 是刚解出的目录树（这时它与源镜像一一对应；要在装内核 / agent 之前调）。
/// 约束：清单来自目录树，属性值读源镜像；本镜像实测约 170 条（21 个特权位 + 140 个非 0 uid + 156 个非 0 gid）。
pub(crate) fn guest_attrs_to_restore(image: &Path, tree: &Path) -> Result<Vec<GuestAttr>, String> {
    let dirs = image_dirs(tree)?;
    let all = image_attrs(image, &dirs)?;
    Ok(all
        .into_iter()
        .filter(|a| a.mode & 0o7000 != 0 || a.uid != 0 || a.gid != 0)
        .collect())
}

/// 目的：把源镜像里那些"打包会丢"的属性回填到最终镜像，再读一遍核对（不一致就硬失败）。
/// 约束：只改需要改的字段（mode 只在有特权位时写、uid / gid 只在非 0 时写）；`sif` 单条失败时
///   debugfs 的退出码未必非零，所以必须自己核对——不把不合规的镜像交给用户。
pub(crate) fn restore_guest_image_attrs(image: &Path, attrs: &[GuestAttr]) -> Result<(), String> {
    if attrs.is_empty() {
        return Ok(());
    }
    let mut cmds = String::new();
    for a in attrs {
        let quoted = debugfs_quote(&a.path)?;
        if a.mode & 0o7000 != 0 {
            cmds.push_str(&format!("sif {} mode 0{:o}\n", quoted, a.mode));
        }
        if a.uid != 0 {
            cmds.push_str(&format!("sif {} uid {}\n", quoted, a.uid));
        }
        if a.gid != 0 {
            cmds.push_str(&format!("sif {} gid {}\n", quoted, a.gid));
        }
    }
    run_debugfs(image, &cmds, true)?;
    let mut parents: Vec<String> = attrs.iter().map(|a| parent_of(&a.path)).collect();
    parents.sort();
    parents.dedup();
    let now = image_attrs(image, &parents)?;
    for want in attrs {
        let Some(got) = now.iter().find(|n| n.path == want.path) else {
            return Err(format!(
                "回填后镜像里找不到 {}（打包链路漏了它）",
                want.path
            ));
        };
        if got.mode != want.mode || got.uid != want.uid || got.gid != want.gid {
            return Err(format!(
                "回填没生效：{} 源是 mode {:o} {}/{}，镜像是 mode {:o} {}/{}",
                want.path, want.mode, want.uid, want.gid, got.mode, got.uid, got.gid
            ));
        }
    }
    Ok(())
}

/// 目的：读一个 ext4 镜像里若干目录的直接条目（`ls -l -p`，一个 debugfs 会话读完）。
/// 参数：dirs = 镜像内绝对路径（`/` 分隔）；实测 2377 个目录一个会话 0.2 秒。
/// 约束：只列给定目录、不递归；路径写进命令表前用双引号包起来（含引号 / 反斜杠 / 换行的直接拒绝）。
pub(crate) fn image_attrs(image: &Path, dirs: &[String]) -> Result<Vec<GuestAttr>, String> {
    if dirs.is_empty() {
        return Ok(Vec::new());
    }
    let mut cmds = String::new();
    for d in dirs {
        cmds.push_str("ls -l -p ");
        cmds.push_str(&debugfs_quote(d)?);
        cmds.push('\n');
    }
    let out = run_debugfs(image, &cmds, false)?;
    let mut attrs = Vec::new();
    let mut dir = String::new();
    for line in out.lines() {
        if let Some(echo) = line.strip_prefix("debugfs: ls -l -p ") {
            dir = unquote(echo.trim())
                .ok_or_else(|| format!("看不懂 debugfs 的命令回显（不猜路径）：{}", line))?;
            continue;
        }
        if dir.is_empty() || !line.starts_with('/') {
            continue;
        }
        // -p 格式：/inode/mode/uid/gid/name/size/ ——名字里的空格原样保留，/ 不可能出现在名字里。
        let parts: Vec<&str> = line.splitn(7, '/').collect();
        if parts.len() < 7 {
            continue;
        }
        let mode = u32::from_str_radix(parts[2], 8)
            .map_err(|_| format!("看不懂 debugfs 的 mode：{}", line))?;
        let uid = parts[3]
            .parse::<u32>()
            .map_err(|_| format!("看不懂 debugfs 的 uid：{}", line))?;
        let gid = parts[4]
            .parse::<u32>()
            .map_err(|_| format!("看不懂 debugfs 的 gid：{}", line))?;
        let name = parts[5];
        if name == "." || name == ".." {
            continue;
        }
        attrs.push(GuestAttr {
            path: format!("{}/{}", dir.trim_end_matches('/'), name),
            mode,
            uid,
            gid,
        });
    }
    Ok(attrs)
}

/// 目的：目录树里的全部目录（镜像内绝对路径，`/` 分隔）；不跟软链（避免环）。
pub(crate) fn image_dirs(tree: &Path) -> Result<Vec<String>, String> {
    let mut out = vec!["/".to_string()];
    let mut stack = vec![(tree.to_path_buf(), String::new())];
    while let Some((abs, rel)) = stack.pop() {
        let entries =
            fs::read_dir(&abs).map_err(|e| format!("读 {} 失败：{}", abs.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("读 {} 失败：{}", abs.display(), e))?;
            let kind = entry
                .file_type()
                .map_err(|e| format!("读 {} 的类型失败：{}", entry.path().display(), e))?;
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|_| {
                format!(
                    "目录名不是 UTF-8（写不进 debugfs 命令表）：{}",
                    entry.path().display()
                )
            })?;
            let child = if rel.is_empty() {
                name
            } else {
                format!("{}/{}", rel, name)
            };
            out.push(format!("/{}", child));
            stack.push((entry.path(), child));
        }
    }
    Ok(out)
}

/// 目的：跑一个 debugfs 会话（`-f` 命令表），返回 stdout；`write` 决定要不要 `-w`。
pub(crate) fn run_debugfs(image: &Path, cmds: &str, write: bool) -> Result<String, String> {
    let cmd_file = image.with_extension(format!("debugfs-{}.cmd", std::process::id()));
    fs::write(&cmd_file, cmds).map_err(|e| format!("写 debugfs 命令表失败：{}", e))?;
    let mut cmd = std::process::Command::new("debugfs");
    if write {
        cmd.arg("-w");
    }
    let out = cmd.arg("-f").arg(&cmd_file).arg(image).output();
    let _ = fs::remove_file(&cmd_file);
    let out = out.map_err(|e| format!("跑不了 debugfs（要 e2fsprogs）：{}", e))?;
    if !out.status.success() {
        return Err(format!(
            "debugfs 失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 目的：把镜像内路径变成 debugfs 命令表里一个双引号参数。
/// 约束：debugfs 命令表按行读、参数用双引号包；会破坏命令的字符直接拒绝（不猜、不静默改名）。
pub(crate) fn debugfs_quote(path: &str) -> Result<String, String> {
    if path.chars().any(|c| matches!(c, '"' | '\\' | '\n' | '\r')) {
        return Err(format!("路径里有 debugfs 命令表放不下的字符：{}", path));
    }
    Ok(format!("\"{}\"", path))
}

/// 目的：去掉 debugfs 回显里的双引号（我们生成的命令不带转义）。
pub(crate) fn unquote(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_string())
}

/// 目的：镜像内路径的父目录（根下的条目归 `/`）。
pub(crate) fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((p, _)) => p.to_string(),
    }
}

/// 目的：目录里所有普通文件的大小之和（不含软链本身）；算镜像要多大。
pub(crate) fn dir_bytes(dir: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).map_err(|e| format!("读 {} 失败：{e}", d.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("看 {} 失败：{e}", path.display()))?;
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total += file_footprint(&meta);
            }
        }
    }
    Ok(total)
}

/// 目的：一个普通文件实际要占的字节数——Unix 上按块算（st_blocks * 512，含块对齐与稀疏），
///   比 len() 更接近 ext4 真正要分配的空间。
/// 约束：非 Unix 拿不到块数，退回文件长度；打包本身只在 Linux 上真跑。
pub(crate) fn file_footprint(meta: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        meta.len()
    }
}

/// 目的：只留下 `keep` 里那几个名字，其余全删（虚拟根目录的收口）。
pub(crate) fn purge_except(base: &Path, keep: &[&str]) -> Result<(), String> {
    let entries = fs::read_dir(base).map_err(|e| format!("读 {} 失败：{e}", base.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let meta =
            fs::symlink_metadata(&path).map_err(|e| format!("看 {} 失败：{e}", path.display()))?;
        if meta.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| format!("删 {} 失败：{e}", path.display()))?;
        } else {
            fs::remove_file(&path).map_err(|e| format!("删 {} 失败：{e}", path.display()))?;
        }
    }
    Ok(())
}
