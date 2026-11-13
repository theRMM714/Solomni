//! 目的：便携 PATH 的那一段：把 <root>/bin 追加进用户 PATH，并支持摘除（unset）与按备份还原（undo）。
//! 管：分段去重（引号 / 尾斜杠 / 大小写不敏感）、写前备份、类型原样写回、状态事实、退出码。
//! 不管：注册表怎么读写（kernel 的 UserPath 端口）、shim 内容（shim.rs）。
//! 联动：entry::setup::run 分派；测试用替身端口，不碰真实 HKCU。

// 本批只实现 Windows 的用户 PATH 改写；其它平台只打印指引（缺口见 tests/gaps.yaml 的 portable-setup.posix），
// 纯分段函数留着给 POSIX 批次与跨平台单测复用，所以在非 Windows 构建上如实放行死代码。
#![cfg_attr(not(windows), allow(dead_code))]

use crate::kernel::ports::{PathValue, UserPath};
use std::path::{Path, PathBuf};

/// 目的：备份文件名（放产品私有区 .home；记为「我们第一次改之前」的原串与原类型）。
pub(crate) const BACKUP_FILE: &str = "setup-path.json";

/// 目的：备份内容：undo 依据它还原原串与原类型。
/// 约束：v 固定写 1（将来换格式时靠它区分）；at 记 Unix 秒（不引本地时区）。
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathBackup {
    pub(crate) v: u8,
    pub(crate) original: String,
    pub(crate) expand: bool,
    pub(crate) bin: String,
    pub(crate) at: u64,
}

/// 目的：PATH 与 shim 的可观察状态（--setup=status 的 path 段）。
#[derive(serde::Serialize)]
pub(crate) struct PathInfo {
    pub(crate) supported: bool,
    pub(crate) bin: String,
    pub(crate) has_bin: bool,
    pub(crate) shim: String,
    pub(crate) shim_present: bool,
    pub(crate) shim_current: bool,
    pub(crate) backup_present: bool,
}

/// 目的：一段 PATH 的规范化形态：去引号、去尾部分隔符、大小写不敏感。
/// 约束：只用于比较；写回时原文逐段保留。
pub(crate) fn norm(seg: &str) -> String {
    seg.trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// 目的：<root>/bin 在 PATH 里的写法（不带尾斜杠）。
pub(crate) fn bin_text(root: &Path) -> String {
    root.join("bin").display().to_string()
}

/// 目的：PATH 里是否已有我们那段。
pub(crate) fn contains(text: &str, bin: &str) -> bool {
    let want = norm(bin);
    text.split(';').any(|seg| norm(seg) == want)
}

/// 目的：把 bin 追加到 PATH 末尾；已有的重复段先摘掉（幂等：重复执行只保留一段）。
/// 约束：其它段原样保留；一个多余的尾空段去掉（避免产生 ;;）。
pub(crate) fn append(text: &str, bin: &str) -> String {
    let want = norm(bin);
    let mut parts: Vec<String> = text
        .split(';')
        .filter(|seg| !seg.is_empty() && norm(seg) != want)
        .map(|seg| seg.to_string())
        .collect();
    if text.trim().is_empty() {
        parts.clear();
    }
    parts.push(bin.to_string());
    parts.join(";")
}

/// 目的：把 bin 从 PATH 里摘掉（所有位置）。返回：新串 + 是否真的摘到了。
pub(crate) fn remove(text: &str, bin: &str) -> (String, bool) {
    let want = norm(bin);
    let mut found = false;
    let parts: Vec<&str> = text
        .split(';')
        .filter(|seg| {
            if norm(seg) == want {
                found = true;
                false
            } else {
                !seg.is_empty()
            }
        })
        .collect();
    (parts.join(";"), found)
}

/// 目的：备份文件路径（备份只在「真的要改 PATH」时写）。
pub(crate) fn backup_path(home: &Path) -> PathBuf {
    home.join(BACKUP_FILE)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_backup(home: &Path) -> Option<PathBackup> {
    std::fs::read_to_string(backup_path(home))
        .ok()
        .and_then(|t| serde_json::from_str::<PathBackup>(&t).ok())
}

/// 目的：把「第一次改之前」的原串落盘（已有备份就不覆盖——undo 永远回到最初那一段）。
fn write_backup(home: &Path, value: &PathValue, bin: &str) -> Result<(), String> {
    let path = backup_path(home);
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("建私有区失败（{}）：{}", dir.display(), e))?;
    }
    let backup = PathBackup {
        v: 1,
        original: value.text.clone(),
        expand: value.expand,
        bin: bin.to_string(),
        at: now_secs(),
    };
    let text = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("写备份失败（{}）：{}", path.display(), e))
}

/// 目的：--setup=path 的核心（端口注入，便于测试）：确保 shim，再把 bin 追加进用户 PATH。
/// 返回：给人看的一行结论。
#[cfg(windows)]
pub(crate) fn do_path(port: &dyn UserPath, home: &Path, root: &Path) -> Result<String, String> {
    let (shim, changed) = crate::entry::setup::shim::ensure(root)?;
    let bin = bin_text(root);
    let current = port.read()?;
    if contains(&current.text, &bin) {
        return Ok(format!(
            "shim {}（{}），用户 PATH 已含 {}（无需改动）",
            shim.display(),
            if changed { "已刷新" } else { "已是最新" },
            bin
        ));
    }
    write_backup(home, &current, &bin)?;
    let next = append(&current.text, &bin);
    port.write(&PathValue {
        text: next,
        expand: current.expand,
    })?;
    let _ = port.broadcast();
    Ok(format!(
        "shim {}（{}）；已把 {} 追加进用户 PATH（{}；新开终端生效）",
        shim.display(),
        if changed { "已写入" } else { "已是最新" },
        bin,
        if current.expand {
            "REG_EXPAND_SZ"
        } else {
            "REG_SZ"
        }
    ))
}

/// 目的：--setup=unset 的核心：只摘我们那段，其它段原样保留；幂等。
pub(crate) fn do_unset(port: &dyn UserPath, root: &Path) -> Result<String, String> {
    let bin = bin_text(root);
    let current = port.read()?;
    let (next, found) = remove(&current.text, &bin);
    if !found {
        return Ok(format!("用户 PATH 未含 {}（无需改动）", bin));
    }
    port.write(&PathValue {
        text: next,
        expand: current.expand,
    })?;
    let _ = port.broadcast();
    Ok(format!("已从用户 PATH 摘掉 {}", bin))
}

/// 目的：--setup=undo 的核心：按备份还原原串与原类型（无备份 = 失败并说明，不猜）。
pub(crate) fn do_undo(port: &dyn UserPath, home: &Path) -> Result<String, String> {
    let Some(backup) = read_backup(home) else {
        return Err(
            "没有 PATH 备份（.home/setup-path.json），无法还原；可先跑 --setup=unset 摘掉那一段"
                .to_string(),
        );
    };
    port.write(&PathValue {
        text: backup.original.clone(),
        expand: backup.expand,
    })?;
    let _ = port.broadcast();
    let _ = std::fs::remove_file(backup_path(home));
    Ok("已按备份还原用户 PATH（原串与原类型）".to_string())
}

/// 目的：状态段（只读；非 Windows 如实标 unsupported）。
#[cfg(windows)]
pub(crate) fn info(root: &Path) -> PathInfo {
    let bin = bin_text(root);
    let (shim_present, shim_current) = crate::entry::setup::shim::inspect(root);
    let port = crate::kernel::detail::WindowsUserPath::new();
    let (supported, has_bin) = match port.read() {
        Ok(value) => (true, contains(&value.text, &bin)),
        Err(_) => (false, false),
    };
    PathInfo {
        supported,
        bin,
        has_bin,
        shim: crate::entry::setup::shim::shim_path(root)
            .display()
            .to_string(),
        shim_present,
        shim_current,
        backup_present: backup_path(&root.join(".home")).exists(),
    }
}

#[cfg(not(windows))]
pub(crate) fn info(root: &Path) -> PathInfo {
    PathInfo {
        supported: false,
        bin: bin_text(root),
        has_bin: false,
        shim: String::new(),
        shim_present: false,
        shim_current: false,
        backup_present: backup_path(&root.join(".home")).exists(),
    }
}

#[cfg(not(windows))]
fn non_windows_guidance(root: &Path) -> i32 {
    println!(
        "[便携] 本平台暂不自动改 PATH（已登记缺口）：请把 {} 加进 shell 的 PATH 即可",
        root.join("bin").display()
    );
    0
}

/// 目的：--setup=path 命令入口：Windows 走端口；其它平台只打印指引（本批不写 POSIX 启动脚本）。
#[cfg(windows)]
pub(crate) fn command(root: &Path) -> i32 {
    let port = crate::kernel::detail::WindowsUserPath::new();
    match do_path(&port, &root.join(".home"), root) {
        Ok(line) => {
            println!("[便携] {}", line);
            0
        }
        Err(e) => {
            eprintln!("[便携] 改用户 PATH 失败：{}", e);
            1
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn command(root: &Path) -> i32 {
    non_windows_guidance(root)
}

/// 目的：--setup=unset 命令入口。
#[cfg(windows)]
pub(crate) fn unset_command(root: &Path) -> i32 {
    let port = crate::kernel::detail::WindowsUserPath::new();
    match do_unset(&port, root) {
        Ok(line) => {
            println!("[便携] {}", line);
            0
        }
        Err(e) => {
            eprintln!("[便携] 摘 PATH 失败：{}", e);
            1
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn unset_command(root: &Path) -> i32 {
    non_windows_guidance(root)
}

/// 目的：--setup=undo 命令入口。
#[cfg(windows)]
pub(crate) fn undo_command(root: &Path) -> i32 {
    let port = crate::kernel::detail::WindowsUserPath::new();
    match do_undo(&port, &root.join(".home")) {
        Ok(line) => {
            println!("[便携] {}", line);
            0
        }
        Err(e) => {
            eprintln!("[便携] 还原 PATH 失败：{}", e);
            1
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn undo_command(root: &Path) -> i32 {
    non_windows_guidance(root)
}
