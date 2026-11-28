//! 目的：执行环境的就绪判定形状——**定义在纯层**（`pure/exec_model/domain/need.rs`），这里只做本单元的入站再导出。
//! 管：`Requirement`（一项前置：事实 + 补法）、`Readiness`（一份声明的逐项前置）、`Machine`（与声明无关的机器能力）、`PolicyView`（策略自我介绍 + 机器能力）。
//! 不管：形状本身怎么定义（纯层）；缺什么怎么办的实现（`vm` 给清单、`provision` 实现 `Fixer`）。
//! 联动：`SandboxPolicy` 的默认方法返回这里；`SandboxPolicies` 汇总给 `work` 与呈现层。

pub use crate::units::pure::exec_model::api::{Machine, Readiness, Requirement};

use crate::units::business::sandbox::domain::facts::PolicyDecl;

/// 目的：一条可选策略的全貌（自我介绍 + 这台机器上的前置）：呈现层的选择器照抄，不按 id 编话。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicyView {
    /// 目的：策略的自我介绍（id、名字、说明、参数声明）。
    pub decl: PolicyDecl,
    /// 目的：这台机器上这个策略的前置要求（含每项怎么补）。
    pub machine: Machine,
}
