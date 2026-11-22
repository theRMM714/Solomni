//! 目的：守门进程——把围栏装进工具进程，然后才跑模块声明的命令（实现 tools 的围栏策略）。
//! 管：把**意图**（FencePlan：哪些路径什么权限、网络开不开）交给平台后端去落成该平台的机制，
//!   并给出「本机能力」与「本次实际」（effective_paths）的同一份判据。
//!   平台差异全部收在 trait Confine 的四个实现里（linux.rs / macos.rs / windows.rs / other.rs）：
//!   上层只认计划，不认 Landlock / seatbelt / AppContainer。
//! 不管：策略（哪些根可达、放不放网）——由 runtime 派生后经命令行传入。
//! 联动：能力不足时如实上报（capability），降级而非崩溃——绝不静默假装有围栏。

use crate::kernel::contracts::fence::{FencePlan, FenceRunPolicy};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod other;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use other as platform;

mod job;
pub(crate) mod prep;

#[cfg(test)]
mod tests;

/// 目的：守门模式参数：主程序带它启动 = 以守门进程身份执行（内部协议，用户不直接用）。
pub const FENCE_FLAG: &str = "--fence-run";
pub use crate::kernel::contracts::fence::FENCE_FAILED;

/// 目的：本机能不能强制住这次执行的围栏——**机制层的验证结论**，与"外层授权了没有"无关。
///   三态的理由：把"本机不允许"与"我们的机制写错了"分开。
///   前者是环境结论，如实降级照跑（能力等级已在启动报告里说过）；后者绝不能静默降级——
///   那等于用户以为有围栏、实际什么都没有。探针早就按这两类分别处理，运行期也必须一样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceVerdict {
    /// 机制真装上了，强制生效。
    Enforced,
    /// 本机环境不允许装（内核不支持、私有 ABI 失效、系统拒绝建容器）——降级照跑，不是我们的错。
    EnvUnavailable(String),
    /// 自检已确认机制有效，但我们的规则/步骤装不上 = 我们写错了。未授权时据此**拒绝执行**。
    Broken(String),
}

/// 目的：测试专用的注入开关：让探针能确定性地构造「本机不允许」这一态。
///   为什么需要：`EnvUnavailable` 只在老内核 / 失效的私有 ABI / 环境拒绝建容器时出现，
///   正常 runner 上碰不到——没有这条开关，那一路分支就只被偶然验证过。
///   **只在测试里打开**（探针自己给守门进程带这个环境变量），运行期永不设置它。
pub const SELFCHECK_FAIL_FLAG: &str = "SOLOMNI_FENCE_SELFCHECK_FAIL";

/// 目的：自检该不该按「本机不允许」处理：测试专用注入优先，否则问真实自检。
///   三平台的自检入口共用这一份判断，避免各写一遍导致探针在某平台上失效。
pub(crate) fn selfcheck_forced_unavailable() -> bool {
    std::env::var_os(SELFCHECK_FAIL_FLAG).is_some()
}

/// 目的：本机后端——三平台各一个实现，上层只通过这个 trait 说话。
/// 约束：实现只做**本平台机制的落成**，不判断该不该授予（那是策略层的事，计划里已经定死）；
///   各自的独有步骤（Windows 的 ACL 授权、Linux 的 seccomp 规则）都留在实现内部，不出现在这里：
///   「先写 ACL」只有 Windows 有，所以它是 Windows 后端自己的入口（`confine::windows::prepare_fence`），
///   不占这个 trait 的位置；调用点都在 Windows 专用文件里，公共路径上没有平台分支。
pub(crate) trait Confine: Sync {
    /// 目的：本机能强制的围栏等级（启动报告用）。
    fn capability(&self) -> Capability;

    /// 目的：机制层自检（不写本机任何权限项）。
    fn verify(&self, plan: &FencePlan, command: &str) -> FenceVerdict;

    /// 目的：本次实际生效的路径级围栏（文件系统, 断网）。
    fn effective_paths(&self, fs: bool, net: bool, write_allowed: bool) -> (bool, bool);

    /// 目的：装围栏 → 跑命令 → 返回退出码。
    fn run(
        &self,
        plan: &FencePlan,
        prepared: Prepared,
        command: &str,
        policy: &FenceRunPolicy,
    ) -> i32;
}

/// 目的：守门进程拿到的「外层已经把本机授权做完了吗」——授权是宿主侧动作，随这次执行交进来。
pub(crate) struct Prepared<'a> {
    /// 目的：外层是否成功写下了本机权限项（失败时按机制结论降级或拒绝）。
    pub prepared: bool,
    /// 目的：产品私有区（台账落点）；探针等没有台账的调用方给 None。
    pub home: Option<&'a Path>,
}

/// 目的：本平台后端（静态单例：后端无状态，机制状态都在系统里）。
pub(crate) fn backend() -> &'static dyn Confine {
    &platform::Backend
}

/// 目的：本机能不能强制住这次执行的围栏（机制层自检，**不写本机任何权限项**）。
///   未授权时段靠它把「环境不允许」与「我们写错了」分开——后者绝不能被当成降级吞掉。
pub fn verify(spec: &FencePlan, command: &str) -> FenceVerdict {
    if selfcheck_forced_unavailable() {
        return FenceVerdict::EnvUnavailable(format!(
            "{}（测试注入：按本机不允许处理）",
            SELFCHECK_FAIL_FLAG
        ));
    }
    backend().verify(spec, command)
}

/// 目的：围栏授权释放的适配器（实现 kernel 共享的 FenceHost 端口）：runtime 只说「这个会话的围栏撤掉」。
pub struct FenceHostAdapter {
    /// 产品私有区（.home/）：台账落点，也是产品根的锚（收尾还原要读它）。
    home: PathBuf,
}

impl FenceHostAdapter {
    /// 目的：组合根注入产品私有区；释放时据此按台账还原或精确撤权。
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}

impl crate::kernel::ports::FenceHost for FenceHostAdapter {
    /// 目的：按会话租约撤销本进程为该会话写下的全部授权与 profile（会话改配 / 删除时调用）。
    fn release_by_lease(&self, lease: &str) -> Result<(), String> {
        crate::kernel::detail::ledger::release_by_lease(&self.home, lease)
    }

    /// 目的：一次**一次性调用**的收尾——撤它自己的 ACE、删它的按次 profile、删 `private/Packages/<名字>` 空壳。
    /// 约束：三步各自记错、合并返回；profile 删不掉时临时目录仍要尽力清（它只是宿主侧的空壳）。
    fn release_call(&self, spec: &FencePlan) -> Result<(), String> {
        crate::kernel::detail::ledger::release_call(spec, &self.home)
    }
}

/// 目的：本机能强制的围栏等级（如实上报给用户与日志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// 目的：文件系统可达范围是否被真正强制。
    pub fs: bool,
    /// 目的：出站网络是否受围栏控制（按 `net` 断/放）。
    pub net: bool,
    /// 目的：进程树是否连根围住（超时/退出能杀整棵）。
    pub tree: bool,
    /// 目的：如实说明（机制名 + 限制）。
    pub note: String,
}

/// 目的：本机能力（装配期如实告知）。
pub fn capability() -> Capability {
    backend().capability()
}

impl Capability {
    /// 目的：本次实际生效的路径级围栏（文件系统, 断网）——本机能力 × 授权。
    ///   只有有「外层写 ACL」这一步的平台依赖授权；其余平台的围栏在守门进程里自足。
    ///   进程树围栏不受授权影响，调用方直接用 `Capability::tree`。
    pub fn effective_paths(&self, write_allowed: bool) -> (bool, bool) {
        backend().effective_paths(self.fs, self.net, write_allowed)
    }
}

pub use crate::kernel::detail::launch::{fence_env, shell_command};
/// 目的：台账条目的归属（测试按它组装样本，见 `tests.rs`）。
#[cfg(test)]
pub use crate::kernel::detail::ledger::Owner;
#[cfg(all(windows, test))]
pub(crate) use crate::kernel::detail::ledger::release_fence;
pub use job::{FenceJob, launcher, run_fenced};
#[cfg(windows)]
pub use prep::FencePrep;
pub use prep::FenceStep;
#[cfg(all(test, windows))]
pub use prep::clean;
pub use prep::{machine_ensure, machine_missing, retire_legacy_machine_grants};

pub(crate) use crate::kernel::detail::acl::interpreter_dir_usable;
#[cfg(windows)]
pub(crate) use crate::kernel::detail::launch::NODE_REALPATH_FLAGS;
pub(crate) use crate::kernel::detail::launch::interpreter_roots_for_exe;
