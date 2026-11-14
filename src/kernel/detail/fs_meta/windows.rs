//! 目的：`fs_meta` 的 Windows 后端——可执行性按扩展名，软链不可建，权限位不适用。
//! 管：可执行判定、执行位空操作、私密权限空操作、占用按逻辑长度、重解析点判定。
//! 不管：Unix 模式位与软链语义；调用方的错误文案。
//! 联动：由 `src/kernel/detail/fs_meta.rs` 按平台选中。

use std::fs;
use std::io;
use std::path::Path;

/// 重解析点属性位：符号链接、目录联接与其它重解析点都置位。
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

/// 目的：是文件即可执行（Windows 的可执行性由扩展名决定）。
pub(crate) fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// 目的：Windows 没有模式位，空操作。
pub(crate) fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// 目的：Windows 没有模式位，空操作。
pub(crate) fn ensure_executable(_path: &Path) {}

/// 目的：Windows 上如实报不支持软链。
pub(crate) fn symlink(_target: &str, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "本平台不支持软链",
    ))
}

/// 目的：Windows 的 ACL 不由模式位表达，空操作。
pub(crate) fn restrict_private_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// 目的：Windows 按逻辑长度报占用。
pub(crate) fn file_footprint(meta: &fs::Metadata) -> u64 {
    meta.len()
}

/// 目的：按重解析点属性判定链接 / 联接（不跟随）。
pub(crate) fn is_link(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

/// 目的：写文件；Windows 不设执行位。
pub(crate) fn write_executable(path: &Path, body: &str) -> io::Result<()> {
    fs::write(path, body)
}

/// 目的：Windows 的目录项刷新没有对应动作，空操作。
pub(crate) fn sync_dir(_path: &Path) {}
