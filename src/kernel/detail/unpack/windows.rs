//! 目的：`unpack` 的 Windows 后端——没有 Unix 权限位，软链按目录 / 文件分开建。
//! 管：归档权限位（本平台不适用）与软链尝试。
//! 不管：归档遍历、路径安全与复制兜底（在门面）。
//! 联动：由 `src/kernel/detail/unpack.rs` 按平台选中。

use std::path::Path;

/// 目的：Windows 没有归档里的 Unix 权限位。
pub(super) fn tar_mode(_header: &tar::Header) -> Option<u32> {
    None
}

/// 目的：Windows 没有归档里的 Unix 权限位。
pub(super) fn zip_mode<R: std::io::Read + ?Sized>(
    _entry: &zip::read::ZipFile<'_, R>,
) -> Option<u32> {
    None
}

/// 目的：Windows 不设权限位。
pub(super) fn apply_mode(_path: &Path, _mode: Option<u32>) {}

/// 目的：先试真链接（目录 / 文件分开建）；成不了交给复制兜底。
pub(super) fn try_symlink(target: &Path, tgt_abs: &Path, out: &Path) -> bool {
    let r = if tgt_abs.is_dir() {
        std::os::windows::fs::symlink_dir(target, out)
    } else {
        std::os::windows::fs::symlink_file(target, out)
    };
    if r.is_ok() {
        return true;
    }
    super::copy_if_exists(tgt_abs, out)
}
