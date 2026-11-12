#![cfg(windows)]
//! 目的：声明 Windows 的 AppContainer 文件系统/网络围栏与 Job Object 进程树围栏真机验收目标（L3）。
//! 管：probe 与 probes 的挂载。
//! 不管：其它平台与跨平台断言。
//! 联动：src/guard/。

#[path = "../helpers/probe.rs"]
mod probe;

mod probes;
