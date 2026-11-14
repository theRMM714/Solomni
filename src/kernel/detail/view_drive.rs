//! 目的：视图盘模块的门面——只有 Windows 有实现，其余平台为空模块。
//! 管：按平台选中 `view_drive/windows.rs` 并重导出其公开项。
//! 不管：盘符分配、路径编解码与登记（在 `windows.rs`）；非 Windows 的恒等 `view_codec`（在 `kernel::api`）。
//! 联动：`kernel::api` 的 Windows 重导出与 `confine/windows` 的容器 cwd 使用。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
