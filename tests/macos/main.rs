#![cfg(target_os = "macos")]
//! macOS 测试目标（L3）：本平台围栏与进程机制的真机验收。

#[path = "../helpers/probe.rs"]
mod probe;

mod probes;
