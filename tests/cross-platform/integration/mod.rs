//! 目的：挂载 L2 集成测试：真进程、真文件系统、本地环回 HTTP。
//! 管：该目录下的集成测试子模块。
//! 不管：平台特权或特定平台机制。
//! 联动：docs/testing/levels.md、tests/helpers。

mod fence_launcher;
mod host_packages;
