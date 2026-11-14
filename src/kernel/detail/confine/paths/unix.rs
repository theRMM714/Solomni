//! 目的：`paths` 的非 Windows 后端——执行位、链接与真身、`sh -c`。
//! 管：解释器安装根、目录可用性、执行位判定与 shell 选择。
//! 不管：围栏授权与台账；环境白名单的公共部分（在门面）。
//! 联动：由 `src/kernel/detail/confine/paths.rs` 在非 Windows 目标选中。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 目的：是文件且有任一执行位；没有执行位概念的目标按「是文件」。
#[cfg(unix)]
pub(super) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// 目的：没有执行位概念的目标按「是文件」。
#[cfg(not(unix))]
pub(super) fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// 目的：解释器程序 → 要授权的安装根（Unix 链接与真身都留）。
pub(super) fn interpreter_roots_for_exe(exe: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let parent = exe.parent();
    let real = std::fs::canonicalize(exe)
        .ok()
        .map(|p| crate::kernel::domain::path::native(&p));
    if let Some(p) = parent {
        dirs.push(super::install_dir(p));
    }
    if let Some(p) = real
        .as_deref()
        .filter(|real| *real != exe)
        .and_then(|real| real.parent())
    {
        dirs.push(super::install_dir(p));
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// 目的：其它平台的围栏只读放行，目录存在即可。
pub(super) fn interpreter_dir_usable(_dir: &Path) -> Result<(), String> {
    Ok(())
}

/// 目的：非 Windows 的工具进程用 `sh -c` 解释命令行。
pub(super) fn shell_command(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// 目的：非 Windows 没有要改写的 PATH 项与 node 开关。
pub(super) fn adjust_env(_out: &mut Vec<(OsString, OsString)>, _command: &str) {}
