//! 目的：vm 能力的入站面——其它能力与呈现层只准用这里。
//! 管：Vm 用例 trait（运行包库 / 包库目录）与档位、计划、诊断、就绪检测的重导出。
//! 不管：纯逻辑实现（domain/）与端口（ports.rs）。
//! 联动：被 conductor / collab / cli 与入口层消费。

pub use crate::capabilities::vm::domain::exec::{
    absent, declared, diagnose_text, plan, plan_summary, tier_readiness, tier_refusal, unavailable,
    vm_diagnoses, vm_requirements, Diagnosis, VmInputs, VmRequirement,
};
pub use crate::capabilities::vm::domain::packages::Library;

/// 目的：虚拟机档的**用例面**（`service.rs` 实现）——运行包库与包库目录走这里。
/// 约束：出站端口 `PackageSource` 只由 `service.rs` 持有（R12）。
pub trait Vm: Send + Sync {
    /// 运行包库：可用的包清单（每次重扫）。
    fn library(&self) -> Library;
    /// 包库所在目录（界面上要如实告诉用户"把包放哪儿"）。
    fn runtimes_dir(&self) -> std::path::PathBuf;
}
