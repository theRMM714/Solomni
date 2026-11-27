//! 目的：沙箱单元的入站面——声明、事实、注册表与策略端口。
//! 管：对其它单元与呈现层暴露的形状与入口（`Side` 复用 `exec_model` 的那一份）。
//! 不管：实现（在各策略所在的单元）；装配（组合根）。
//! 联动：`session_model` 存 `SandboxDecl`；`work` 与呈现层读 `SandboxFacts` / `PolicyDecl`。

#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::decl::{HOST_POLICY, SandboxDecl, VM_POLICY};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::policy::SandboxPolicy;
#[allow(unused_imports)]
pub use crate::units::business::sandbox::service::SandboxPolicies;

use std::sync::Arc;

/// 目的：本机策略的实例（注册表的第一项，也是没有客机时的缺省）。
pub fn host_policy() -> Arc<dyn SandboxPolicy> {
    Arc::new(crate::units::business::sandbox::domain::host::HostPolicy)
}
