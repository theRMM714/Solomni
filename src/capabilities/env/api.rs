//! 目的：env 能力的入站契约：环境事实的类型与派生函数唯一出口。
//! 管：重导出 `domain::env` 的五个派生函数与配套事实结构。
//! 不管：实现（`domain/env.rs`）；取事实（`conductor` 的 service 层）。
//! 联动：`conductor` 与 `src/tests/env.rs` 只经本模块取用。

#[allow(unused_imports)] // 入站面：事实类型给调用方标注与测试取用
pub use crate::capabilities::env::domain::env::{
    MemberEnv, MemberFacts, SandboxesFacts, member, permissions, sandboxes, workspace_grant,
};
