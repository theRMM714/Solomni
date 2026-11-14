//! 目的：宿主文件元数据的平台原语——可执行位、软链、私密文件权限、占用估算与链接判定。
//! 管：按平台读 / 写文件模式与链接；调用方只认这里的函数名。
//! 不管：业务语义与错误文案（调用方决定）；目录创建与路径翻译。
//! 联动：由 `src/kernel/api.rs` 重导出；provision / registry / secrets / workspace 的 detail 使用。

use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(any(unix, windows)))]
mod other;
#[cfg(not(any(unix, windows)))]
use other as backend;

/// 目的：文件是否可执行。
pub fn is_executable(path: &Path) -> bool {
    backend::is_executable(path)
}

/// 目的：给文件补上执行位（不具备该概念的平台为空操作）。
pub fn make_executable(path: &Path) {
    let _ = backend::make_executable(path);
}

/// 目的：给文件补上执行位并如实返回失败（需要向用户报错的调用方用它）。
pub fn make_executable_checked(path: &Path) -> io::Result<()> {
    backend::make_executable(path)
}

/// 目的：给已落盘文件补齐执行位；不是文件就不做。
pub fn ensure_executable(path: &Path) {
    backend::ensure_executable(path)
}

/// 目的：建软链；平台没有该机制时返回 `Unsupported`。
pub fn symlink(target: &str, link: &Path) -> io::Result<()> {
    backend::symlink(target, link)
}

/// 目的：把文件收紧为「仅所有者可读写」（不具备该概念的平台为空操作）。
pub fn restrict_private_file(path: &Path) -> io::Result<()> {
    backend::restrict_private_file(path)
}

/// 目的：宿主上的实际占用字节（Unix 按块；其余按逻辑长度）。
pub fn file_footprint(meta: &fs::Metadata) -> u64 {
    backend::file_footprint(meta)
}

/// 目的：路径本身是不是链接 / 联接（不跟随）。
pub fn is_link(path: &Path) -> bool {
    backend::is_link(path)
}

/// 目的：写一个可执行脚本（Unix 设 0755；其余平台只写文件）。
pub fn write_executable(path: &Path, body: &str) -> io::Result<()> {
    backend::write_executable(path, body)
}

/// 目的：把目录项刷到盘上（Unix 上 rename 后需要它；其余平台为空操作，尽力而为）。
pub fn sync_dir(path: &Path) {
    backend::sync_dir(path);
}
