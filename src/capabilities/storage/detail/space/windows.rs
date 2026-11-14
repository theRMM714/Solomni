//! 目的：`space` 的 Windows 后端——`GetDiskFreeSpaceExW` 读卷总量与可用量。
//! 管：路径所在的卷的 (total, free) 字节。
//! 不管：预算、回收与文案。
//! 联动：由 `src/capabilities/storage/detail/space.rs` 按平台选中。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

/// 目的：`GetDiskFreeSpaceExW` 的薄封装；调用失败给 None。
pub(crate) fn total_free(path: &Path) -> Option<(u64, u64)> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
    if ok == 0 { None } else { Some((total, free)) }
}
