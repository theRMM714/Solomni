//! 目的：`fs_meta` 的兜底后端——没有模式位与软链语义的平台如实降级。
//! 管：可执行判定按是文件、执行位空操作、软链报不支持、私密权限空操作、占用按长度。
//! 不管：Windows / Unix 的具体机制。
//! 联动：由 `src/kernel/detail/fs_meta.rs` 在非 Unix / 非 Windows 目标选中。

use std::fs;
use std::io;
use std::path::Path;

/// 目的：是文件即可执行。
pub(crate) fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// 目的：没有模式位，空操作。
pub(crate) fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// 目的：没有模式位，空操作。
pub(crate) fn ensure_executable(_path: &Path) {}

/// 目的：本平台不支持软链。
pub(crate) fn symlink(_target: &str, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "本平台不支持软链",
    ))
}

/// 目的：没有权限位概念，空操作。
pub(crate) fn restrict_private_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// 目的：按逻辑长度报占用。
pub(crate) fn file_footprint(meta: &fs::Metadata) -> u64 {
    meta.len()
}

/// 目的：按文件类型判定软链。
pub(crate) fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 目的：写文件；不设执行位。
pub(crate) fn write_executable(path: &Path, body: &str) -> io::Result<()> {
    fs::write(path, body)
}

/// 目的：没有目录项刷新动作，空操作。
pub(crate) fn sync_dir(_path: &Path) {}
