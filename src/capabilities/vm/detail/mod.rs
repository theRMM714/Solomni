//! 目的：vm 自己的适配器（只能由入口层的组合根构造）。
//! 管：PackageSource 的磁盘实现与 GuestHost 的 QEMU 实现。
//! 不管：纯逻辑（domain/）与端口定义（ports.rs）。
//! 联动：实现 ports.rs 的两个端口；被组合根装配。

pub mod fs_packages;
pub mod qemu;

pub use fs_packages::FsPackages;
pub use qemu::QemuGuest;
