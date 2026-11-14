//! 目的：视图盘生命周期的跨平台门面——分配、清扫、释放与路径编解码入口。
//! 管：按平台把四个动作交给后端；非 Windows 恒为不可用。
//! 不管：盘符分配与登记细节（在 `view_drive/windows.rs`）；调用时机（组合根）。
//! 联动：`src/kernel/api.rs` 重导出；`src/main.rs` 的启动 / 退出钩子与沙箱路径编解码使用。

use std::path::Path;
use std::sync::Arc;

use crate::kernel::ports::PathCodec;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as backend;

/// 目的：分配本实例的视图盘；返回是否真的分配了。
pub fn begin(root: &Path, home: &Path) -> bool {
    backend::begin(root, home)
}

/// 目的：只清扫视图盘的崩溃残留（不分配盘符）。
pub fn sweep(home: &Path) {
    backend::sweep(home)
}

/// 目的：退出时注销本进程的视图盘登记。
pub fn release(home: &Path) {
    backend::release(home)
}

/// 目的：沙箱用的路径编解码器；没有视图盘时给 `None`。
pub fn codec() -> Option<Arc<dyn PathCodec>> {
    backend::codec()
}
