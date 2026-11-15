//! 目的：`path` 的 Windows 后端——写用户 PATH、备份原串、按备份还原。
//! 管：shim 确保、PATH 追加、状态读取与三个命令入口。
//! 不管：分段规则与可移植的 unset / undo（在 `path/rules.rs`）；注册表读写（端口）。
//! 联动：由 `setup/path.rs` 按平台选中。

use super::*;
use crate::kernel::ports::{PathValue, UserPath};

/// 目的：当前 Unix 秒（备份时间戳用）。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
pub(crate) fn do_path(port: &dyn UserPath, home: &Path, root: &Path) -> Result<String, String> {
    let (shim, changed) = crate::capabilities::system::detail::setup::shim::ensure(root)?;
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

/// 目的：状态段（只读）。
pub(crate) fn info(root: &Path) -> PathInfo {
    let bin = bin_text(root);
    let (shim_present, shim_current) =
        crate::capabilities::system::detail::setup::shim::inspect(root);
    let port = crate::kernel::detail::WindowsUserPath::new();
    let (supported, has_bin) = match port.read() {
        Ok(value) => (true, contains(&value.text, &bin)),
        Err(_) => (false, false),
    };
    PathInfo {
        supported,
        bin,
        has_bin,
        shim: crate::capabilities::system::detail::setup::shim::shim_path(root)
            .display()
            .to_string(),
        shim_present,
        shim_current,
        backup_present: backup_path(&root.join(".home")).exists(),
    }
}

/// 目的：path 模式的一句话结果（失败带上下文，交给呈现层排版）。
pub(crate) fn path_line(root: &Path) -> Result<String, String> {
    let port = crate::kernel::detail::WindowsUserPath::new();
    do_path(&port, &root.join(".home"), root).map_err(|e| format!("改用户 PATH 失败：{}", e))
}

/// 目的：unset 模式的一句话结果。
pub(crate) fn unset_line(root: &Path) -> Result<String, String> {
    let port = crate::kernel::detail::WindowsUserPath::new();
    do_unset(&port, root).map_err(|e| format!("摘 PATH 失败：{}", e))
}

/// 目的：undo 模式的一句话结果。
pub(crate) fn undo_line(root: &Path) -> Result<String, String> {
    let port = crate::kernel::detail::WindowsUserPath::new();
    do_undo(&port, &root.join(".home")).map_err(|e| format!("还原 PATH 失败：{}", e))
}
