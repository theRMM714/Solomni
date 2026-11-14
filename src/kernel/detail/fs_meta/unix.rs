//! 目的：`fs_meta` 的 Unix 后端——模式位、软链与块占用。
//! 管：可执行判定与补齐、软链、私密权限（0600）、块占用、链接判定、可执行脚本（0755）。
//! 不管：Windows 的重解析点与 ACL；调用方的错误文案。
//! 联动：由 `src/kernel/detail/fs_meta.rs` 按平台选中。

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

/// 目的：是文件且有任一执行位。
pub(crate) fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

/// 目的：保留现有模式位并补上执行位。
pub(crate) fn make_executable(path: &Path) -> io::Result<()> {
    let mode = fs::metadata(path)?.permissions().mode();
    fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o111))
}

/// 目的：没有执行位的文件补上 0555；已有执行位或不是文件就不做。
pub(crate) fn ensure_executable(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let mode = meta.permissions().mode() & 0o7777;
    if mode & 0o111 != 0 {
        return;
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o555));
}

/// 目的：建 Unix 软链。
pub(crate) fn symlink(target: &str, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// 目的：把文件收紧为 0600。
pub(crate) fn restrict_private_file(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

/// 目的：按实际块占用报字节数。
pub(crate) fn file_footprint(meta: &fs::Metadata) -> u64 {
    meta.blocks().saturating_mul(512)
}

/// 目的：按文件类型判定软链（不跟随）。
pub(crate) fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 目的：写文件并设 0755。
pub(crate) fn write_executable(path: &Path, body: &str) -> io::Result<()> {
    fs::write(path, body)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// 目的：打开目录并 `sync_all`，失败只当尽力而为。
pub(crate) fn sync_dir(path: &Path) {
    if let Ok(dir) = fs::File::open(path) {
        let _ = dir.sync_all();
    }
}
