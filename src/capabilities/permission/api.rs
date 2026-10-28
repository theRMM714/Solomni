//! 目的：权限能力的入站面——别的能力与呈现层只准用这里。
//! 管：重导出权限类型、四档预设与校验入口。
//! 不管：纯规则本身（在 domain）；落盘与求交的调用方（registry / session / conductor）。
//! 联动：workspace、tools、conductor 都经这里消费权限。

#[allow(unused_imports)] // 入站面：其它能力与测试都经这里取类型
pub use crate::capabilities::permission::domain::permission::{
    hit, normalize, validate_defaults, validate_entry, validate_layer, CallScope, Decision,
    DecisionEdit, Decisions, Effect, PermissionDefaults, PermissionLayer, PermissionSet, ProfileId,
    ProfileSpec, Reach, ReachSet, ScopeMode, SessionSandbox, SharedEdit, SharedScope,
};
