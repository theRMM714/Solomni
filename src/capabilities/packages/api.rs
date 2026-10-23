//! 目的：packages 能力的入站面——其它能力与呈现层只准用这里。
//! 管：Packages 用例 trait、包库快照、目标选择、PackagePlan 与包选择诊断。
//! 不管：纯逻辑实现（domain/）与端口（ports.rs）。
//! 联动：被 vm / conductor / diagnostics / residents 消费；契约见 RUNTIME_SPEC.md。

pub use crate::capabilities::packages::domain::manifest::{
    library_for_guest, Library, Target, KIND_SYSTEM,
};
pub use crate::capabilities::packages::domain::plan::{
    absent, build_packages, diagnose_text, diagnoses, plan_guest, plan_host, plan_host_requests,
    requests_for, Facts, PackagePlan, PinPolicy, PlannedPackage, Problem, Provider,
};

/// 目的：运行包库的用例面（service.rs 实现）——包库快照与依赖文件夹路径走这里。
/// 约束：出站端口 PackageSource 只由 service.rs 持有（R12）。
pub trait Packages: Send + Sync {
    /// 运行包库：可用的包清单（每次重扫）。
    fn library(&self) -> Library;
    /// 包库所在目录（界面上要如实告诉用户"把包放哪儿"）。
    fn runtimes_dir(&self) -> std::path::PathBuf;
}
