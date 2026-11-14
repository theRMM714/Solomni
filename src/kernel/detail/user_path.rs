//! 目的：用户 PATH 模块的门面——只有 Windows 有实现，其余平台为空模块。
//! 管：按平台选中 `user_path/windows.rs` 并重导出其公开项。
//! 不管：PATH 该不该改、怎么分段去重（在 entry::setup::path 的纯函数里）。
//! 联动：`src/kernel/detail.rs` 只在 Windows 重导出 `WindowsUserPath`。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
