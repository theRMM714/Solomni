//! 目的：`view_drive_lifecycle` 的非 Windows 后端——没有视图盘，全部如实降级。
//! 管：不分配、不清扫、不释放；路径编解码恒 `None`。
//! 不管：Windows 的盘符分配与登记。
//! 联动：由 `src/kernel/detail/view_drive_lifecycle.rs` 在非 Windows 目标选中。

use std::path::Path;
use std::sync::Arc;

use crate::kernel::ports::PathCodec;

/// 目的：非 Windows 没有视图盘：不分配。
pub(super) fn begin(_root: &Path, _home: &Path) -> bool {
    false
}

/// 目的：非 Windows 没有视图盘可清扫。
pub(super) fn sweep(_home: &Path) {}

/// 目的：非 Windows 没有视图盘可释放。
pub(super) fn release(_home: &Path) {}

/// 目的：非 Windows 路径怎么写就怎么用，恒 `None`。
pub(super) fn codec() -> Option<Arc<dyn PathCodec>> {
    None
}
