//! 目的：视图盘的对外面——把宿主路径映射成容器可用的盘符/挂载，并在用完后收回。
//! 管：`ViewDrive` 与挂载/回收（`active_view` / `attach_view_drive` / `ensure_view_drive` / `host_of` / `view_of` / `release_view_drive` / `sweep_view_drive`），以及生命周期四步（`view_drive_begin` / `view_codec` / `view_drive_release` / `view_drive_sweep`）。
//! 不管：盘符分配与记录台账（在 `detail/view_drive` 与 `detail/view_drive_lifecycle`）。
//! 联动：装配层与守门进程按需取用；内核内部直接取 `detail::view_drive` / `detail::view_drive_lifecycle`。
//!   视图盘是契约的一部分，bin crate 内暂时不会每个都调用到，故对未用到的重导出显式放行；
//!   不从出口删掉——出口少一个，契约就悄悄漂了。

#[cfg(windows)]
#[allow(unused_imports)]
pub use crate::kernel::detail::view_drive::{
    ViewDrive, active_view, attach_view_drive, ensure_view_drive, host_of, release_view_drive,
    sweep_view_drive, view_of,
};
#[allow(unused_imports)]
pub use crate::kernel::detail::view_drive_lifecycle::{
    begin as view_drive_begin, codec as view_codec, release as view_drive_release,
    sweep as view_drive_sweep,
};
