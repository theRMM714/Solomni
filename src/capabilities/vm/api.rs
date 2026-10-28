//! 目的：vm 能力的入站面——其它能力与呈现层只准用这里。
//! 管：Vm 用例 trait（guest 生命周期）与档位、计划、入口诊断、就绪检测的重导出。
//! 不管：纯逻辑实现（domain/）与端口（ports.rs）；运行包契约与选包在 packages 能力。
//! 联动：被 conductor / collab / cli 与入口层消费。

pub use crate::capabilities::vm::domain::exec::resolve_guest;
pub use crate::capabilities::vm::domain::exec::{
    admit_guest, capability_refusal, copy_share_enabled, diagnose_text_for, guest_platform,
    missing_entries_for, plan, plan_summary, runs_on_target, share_chain, tier_readiness,
    tier_refusal, vm_capability, vm_requirements, ExecPlan, MissingEntry, VmInputs, VmRequirement,
    COPY_SHARE_ENV,
};
pub use crate::capabilities::vm::domain::guest::{
    classify_base, mounts_from, package_setup_scripts, GuestImage, GuestRun, GuestSpec,
    OutputLimit, SeedDocs,
};
pub use crate::capabilities::vm::domain::guest::{GuestArch, GuestClients, GuestOs};
pub use crate::capabilities::vm::domain::paths::PathBridge;

/// 目的：虚拟机档的**用例面**（`service.rs` 实现）——guest 生命周期走这里。
/// 约束：出站端口 `VmEngine` / `ShareBackend` 只由 `service.rs` 持有（R12）；运行包库在 packages 能力。
pub trait Vm: Send + Sync {
    /// 按规格拉起会话级 guest（同一租约重复拉起 = 先销毁旧的）。
    fn guest_start(&self, spec: &GuestSpec) -> Result<(), String>;
    /// 在该租约的 guest 里跑一次工具（没有这个 guest = 如实失败）。
    fn guest_run(&self, lease: &str, run: &GuestRun) -> crate::kernel::api::ToolOutcome;
    /// 目的：按某个 agent 的围栏在 guest 里跑一次工具；通道不支持围栏就如实失败。
    fn guest_run_fenced(
        &self,
        lease: &str,
        run: &GuestRun,
        fence: &crate::kernel::api::FenceSpec,
    ) -> crate::kernel::api::ToolOutcome;
    /// 目的：这台 guest 启动时装载的运行包（id@version，排序去重）；没有活 guest = None。
    fn guest_packages(&self, lease: &str) -> Option<Vec<String>>;
    /// 目的：把本次启动实际装载的运行包记进租约（供"新子会话要不要重建"判断）。
    fn guest_set_packages(&self, lease: &str, packages: Vec<String>);
    /// 目的：等 guest 的命令通道就绪（首启装 agent 期间轮询它）。
    /// 返回：到期限仍不通给 false；租约不在场也给 false。
    /// 约束：必须**有界**返回，不能在没有 agent 时干等。
    fn guest_wait_agent(&self, lease: &str, timeout: std::time::Duration) -> bool;
    /// 销毁该租约的 guest 与它的磁盘（幂等）。
    fn guest_stop(&self, lease: &str);
    /// 目的：此刻活着的租约名（并发上限判断用；按名字升序）。
    fn guest_leases(&self) -> Vec<String>;
}
