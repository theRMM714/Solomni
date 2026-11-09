//! 目的：provision 的适配器——把端口落到 ureq、文件系统与系统工具上。
//! 管：每个端口的一份具体实现。
//! 不管：编排顺序与清单语义（在 service）。
//! 联动：组合根构造这些实现，经 ports 交给 service。

pub mod fetch;
pub mod guest_image;
pub mod host;
pub mod install;
pub mod ledger;
pub mod procs;
