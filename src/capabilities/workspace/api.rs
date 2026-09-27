//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::workspace::domain::exec::{
    absent, declared, diagnose_text, plan, plan_summary, tier_readiness, tier_refusal, unavailable,
    vm_diagnoses, vm_requirements, Diagnosis, ExecSpec, VmInputs, VmRequirement,
};
pub use crate::capabilities::workspace::domain::module::{
    agent_system, check_runtimes, check_tools, listing, Module, ModuleManifest, Param, ParamType,
    Roster, ToolDecl,
};
pub use crate::capabilities::workspace::domain::packages::{Library, PackageManifest};
pub use crate::capabilities::workspace::domain::workspace::{
    safe_file_name, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots,
};

/// 工作区的**队列面**：呈现层要的清单事实（模块公地 + 拒收原因）。
/// 只报事实、不做选择——"挑哪些模块组合成一个 agent"是用户与核心的事。
pub trait WorkspaceOps: Send + Sync {
    fn roster(&self) -> Result<Roster, String>;
}
