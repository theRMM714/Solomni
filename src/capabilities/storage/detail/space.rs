//! 目的：宿主磁盘总量与可用量（跨平台，不引入额外 crate）。
//! 管：statvfs / GetDiskFreeSpaceExW 的薄封装；失败给 None。
//! 不管：预算与回收策略。
//! 联动：storage::service 用它算自动上限与最低剩余。

use std::path::Path;

/// 目的：返回 (total, free) 字节；取不到给 None。
pub fn total_free(path: &Path) -> Option<(u64, u64)> {
    platform(path)
}

#[cfg(unix)]
fn platform(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let total = (st.f_blocks as u64).saturating_mul(st.f_frsize as u64);
    let free = (st.f_bavail as u64).saturating_mul(st.f_frsize as u64);
    Some((total, free))
}

#[cfg(windows)]
fn platform(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
    if ok == 0 { None } else { Some((total, free)) }
}

#[cfg(not(any(unix, windows)))]
fn platform(_path: &Path) -> Option<(u64, u64)> {
    None
}
