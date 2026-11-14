//! 目的：`space` 的兜底后端——没有已知的卷统计机制。
//! 管：如实返回「取不到」。
//! 不管：预算、回收与文案。
//! 联动：由 `src/capabilities/storage/detail/space.rs` 在其它平台选中。

use std::path::Path;

/// 目的：未知平台如实返回 None。
pub(crate) fn total_free(_path: &Path) -> Option<(u64, u64)> {
    None
}
