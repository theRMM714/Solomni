//! 目的：`locus` 的非 Windows 后端——设备号 + inode；没有网络路径语义。
//! 管：取落点身份、网络路径判定（恒 false）。
//! 不管：逐级祖先比较（在门面）；路径文本规范化。
//! 联动：由 `src/kernel/detail/locus.rs` 在非 Windows 目标选中。

use crate::kernel::domain::path::Locus;
use std::path::Path;

/// 目的：取路径的落点身份（设备号 + inode）；没有该机制的平台如实报不支持。
#[cfg(unix)]
pub(super) fn locus_of(path: &Path) -> std::io::Result<Locus> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::metadata(path)?;
    Ok(Locus {
        volume: md.dev(),
        id: u128::from(md.ino()),
    })
}

/// 目的：没有落点身份机制的平台如实报不支持。
#[cfg(not(unix))]
pub(super) fn locus_of(_path: &Path) -> std::io::Result<Locus> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "这个平台没有落点身份",
    ))
}

/// 目的：POSIX 的 `//x` 没有网络语义，恒 false。
pub(super) fn network_path(_path: &Path) -> bool {
    false
}
