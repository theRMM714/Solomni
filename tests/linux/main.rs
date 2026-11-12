#![cfg(target_os = "linux")]
//! 目的：声明 Linux 平台围栏真机验收目标（L3）。
//! 管：probe 与 probes 的挂载。
//! 不管：其它平台与跨平台断言。
//! 联动：src/guard/、docs/testing/levels.md。

#[path = "../helpers/probe.rs"]
mod probe;

mod probes;
