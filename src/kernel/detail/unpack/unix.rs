//! 目的：`unpack` 的非 Windows 后端——还原 Unix 权限位与软链。
//! 管：归档权限位读取与写回、软链尝试。
//! 不管：归档遍历、路径安全与复制兜底（在门面）。
//! 联动：由 `src/kernel/detail/unpack.rs` 在非 Windows 目标选中。

use std::path::Path;

/// 目的：读 tar 头里的模式位；没有该概念的平台给 None。
#[cfg(unix)]
pub(super) fn tar_mode(header: &tar::Header) -> Option<u32> {
    header.mode().ok()
}

/// 目的：没有模式位的平台给 None。
#[cfg(not(unix))]
pub(super) fn tar_mode(_header: &tar::Header) -> Option<u32> {
    None
}

/// 目的：读 zip 条目里的 Unix 模式；没有该概念的平台给 None。
#[cfg(unix)]
pub(super) fn zip_mode<R: std::io::Read + ?Sized>(
    entry: &zip::read::ZipFile<'_, R>,
) -> Option<u32> {
    entry.unix_mode()
}

/// 目的：没有模式位的平台给 None。
#[cfg(not(unix))]
pub(super) fn zip_mode<R: std::io::Read + ?Sized>(
    _entry: &zip::read::ZipFile<'_, R>,
) -> Option<u32> {
    None
}

/// 目的：还原权限位（含可执行位）；没有模式位的平台为空操作。
#[cfg(unix)]
pub(super) fn apply_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(m) = mode {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(m & 0o7777));
    }
}

/// 目的：没有模式位的平台为空操作。
#[cfg(not(unix))]
pub(super) fn apply_mode(_path: &Path, _mode: Option<u32>) {}

/// 目的：先试真链接；成不了交给复制兜底。
pub(super) fn try_symlink(target: &Path, tgt_abs: &Path, out: &Path) -> bool {
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(target, out).is_ok() {
            return true;
        }
    }
    super::copy_if_exists(tgt_abs, out)
}
