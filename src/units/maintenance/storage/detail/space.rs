//! 目的：宿主磁盘总量与可用量（跨平台，不引入额外 crate）。
//! 管：按平台选中 statvfs / GetDiskFreeSpaceExW 的薄封装；失败给 None。
//! 不管：预算与回收策略。
//! 联动：storage::service 用它算自动上限与最低剩余。

use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(any(unix, windows)))]
mod other;
#[cfg(not(any(unix, windows)))]
use other as platform;

/// 目的：返回 (total, free) 字节；取不到给 None。
pub fn total_free(path: &Path) -> Option<(u64, u64)> {
    platform::total_free(path)
}
