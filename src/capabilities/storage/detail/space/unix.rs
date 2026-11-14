//! 目的：`space` 的 Unix 后端——`statvfs` 读卷总量与可用量。
//! 管：路径所在的卷的 (total, free) 字节。
//! 不管：预算、回收与文案。
//! 联动：由 `src/capabilities/storage/detail/space.rs` 按平台选中。

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// 目的：`statvfs` 的薄封装；调用失败给 None。
pub(crate) fn total_free(path: &Path) -> Option<(u64, u64)> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let total = (st.f_blocks as u64).saturating_mul(st.f_frsize as u64);
    let free = (st.f_bavail as u64).saturating_mul(st.f_frsize as u64);
    Some((total, free))
}
