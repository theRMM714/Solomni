#![cfg(windows)]
//! Windows 测试目标（L3）：AppContainer 的文件系统与网络围栏、Job Object 进程树围栏。

#[path = "../helpers/probe.rs"]
mod probe;

mod probes;
