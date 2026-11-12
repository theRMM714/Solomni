//! 目的：声明跨平台测试目标（L2 集成 + L4 端到端），断言不依赖特权或特定平台机制。
//! 管：integration 与 helpers 的挂载。
//! 不管：平台机制（围栏、目录授权、断网）——归 tests/<平台>/。
//! 联动：docs/testing/levels.md、tests/cross-platform/。

#[path = "../helpers/probe.rs"]
mod probe;

mod integration;
