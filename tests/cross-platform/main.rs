//! 跨平台测试目标（L2 集成 + L4 端到端）：这里的断言不得依赖任何特权或特定平台的机制，
//! 平台机制（围栏、目录授权、断网……）归 tests/<平台>/ 的探针。见 TESTING.md。

#[path = "../helpers/probe.rs"]
mod probe;

mod integration;
