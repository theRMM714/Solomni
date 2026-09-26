//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::workspace::domain::exec::{
    absent, declared, diagnose_text, plan, plan_summary, tier_readiness, tier_refusal, unavailable,
    vm_diagnoses, vm_requirements, Diagnosis, ExecSpec, VmInputs, VmRequirement,
};
pub use crate::capabilities::workspace::domain::module::{
    agent_system, check_runtimes, listing, Module, ModuleManifest, Param, ParamType, Roster,
    ToolDecl,
};
pub use crate::capabilities::workspace::domain::packages::{Library, PackageManifest};
pub use crate::capabilities::workspace::domain::workspace::{
    safe_file_name, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots,
};
