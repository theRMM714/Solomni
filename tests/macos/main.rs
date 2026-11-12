#![cfg(target_os = "macos")]
//! 目的：声明 macOS 平台围栏与进程机制真机验收目标（L3）。
//! 管：probe 与 probes 的挂载。
//! 不管：其它平台与跨平台断言。
//! 联动：src/guard/。

#[path = "../helpers/probe.rs"]
mod probe;

mod probes;
