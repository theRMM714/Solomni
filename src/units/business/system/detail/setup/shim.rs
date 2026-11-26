//! 目的：便携启动 shim 模块的门面——只有 Windows 有实现，其余平台为空模块。
//! 管：按平台选中 `shim/windows.rs` 并重导出其公开项。
//! 不管：PATH 怎么改（path.rs）、围栏授权（fence.rs）。
//! 联动：path.rs 在改 PATH 前先调用 ensure。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;
