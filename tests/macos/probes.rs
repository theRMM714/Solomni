//! 目的：挂载仅 macOS 可跑的探针。
//! 管：fence 子模块。
//! 不管：其它平台。
//! 联动：tests/macos/。

mod fence;
mod proc_watchdog;

#[path = "../helpers/https_probe.rs"]
mod https;
