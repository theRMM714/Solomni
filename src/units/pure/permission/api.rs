//! 目的：权限单元的入站面——别的单元与呈现层只准用这里。
//! 管：重导出权限类型、两档预设与共享区归属规则。
//! 不管：纯规则本身（在 domain）；落盘与合并的调用方（registry / session / runtime）。
//! 联动：workspace、tools、runtime 都经这里消费权限。

#[allow(unused_imports)] // 入站面：其它单元与测试都经这里取类型
pub use crate::units::pure::permission::domain::permission::{
    PermissionDefaults, PermissionLayer, PermissionOverride, PermissionSet, ProfileId, ProfileSpec,
};

/// 目的：共享区路径归属（读 / 写范围与黑名单）——登记处的默认值与会话树的收窄都只认这一份规则。
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::pure::permission::domain::shared_scope::{
    ScopeMode, SharedEdit, SharedScope, hit, normalize, validate_entry,
};
