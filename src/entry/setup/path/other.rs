//! 目的：`path` 的非 Windows 后端——不自动改 PATH，只打印指引。
//! 管：状态段如实标 unsupported；三个命令入口打印同一份指引。
//! 不管：注册表、shim 与 PATH 改写（Windows 后端）。
//! 联动：由 `src/entry/setup/path.rs` 在非 Windows 目标选中。

use super::*;

/// 目的：状态段（只读；非 Windows 如实标 unsupported）。
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

/// 目的：非 Windows 的统一指引：把 <root>/bin 加进 shell 的 PATH。
fn guidance(root: &Path) -> i32 {
    println!(
        "[便携] 本平台暂不自动改 PATH（已登记缺口）：请把 {} 加进 shell 的 PATH 即可",
        root.join("bin").display()
    );
    0
}

/// 目的：--setup=path 命令入口。
pub(crate) fn command(root: &Path) -> i32 {
    guidance(root)
}

/// 目的：--setup=unset 命令入口。
pub(crate) fn unset_command(root: &Path) -> i32 {
    guidance(root)
}

/// 目的：--setup=undo 命令入口。
pub(crate) fn undo_command(root: &Path) -> i32 {
    guidance(root)
}
