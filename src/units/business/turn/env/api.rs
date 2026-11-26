//! 目的：env 能力的入站契约：环境事实的类型与派生函数唯一出口。
//! 管：重导出 `domain::env` 的派生函数（围栏 / 沙箱 / 权限 / 网络开关 / 宿主只读根）与配套事实结构。
//! 不管：实现（`domain/env.rs`）；取事实（`runtime` 的 service 层）。
//! 联动：`runtime` 与 `src/tests/env.rs` 只经本模块取用。

#[allow(unused_imports)] // 入站面：事实类型给调用方标注与测试取用
pub use crate::units::business::turn::env::domain::env::{
    MemberEnv, MemberFacts, SandboxesFacts, SeatAccess, host_read_roots, member, network_allowed,
    permissions, sandboxes, shared_scope,
};
