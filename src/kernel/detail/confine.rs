//! 目的：守门进程——把围栏装进工具进程，然后才跑模块声明的命令（实现 tools 的围栏策略）。
//! 管：按平台把围栏（可读可写的根、断网、进程树围栏、资源上限）装好，并给出「本机能力」与
//!   「本次实际」（effective_paths）的同一份判据；平台实现分文件 linux.rs / macos.rs / windows/ / other.rs。
//! 不管：策略（哪些根可达、放不放网）——由 conductor 派生后经命令行传入。
//! 联动：能力不足时如实上报（capability），降级而非崩溃——绝不静默假装有围栏。

use crate::kernel::api::FenceSpec;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod other;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use other as backend;

mod job;
mod ledger;
mod paths;
mod prep;
mod texts;

#[cfg(test)]
mod tests;

/// 目的：守门模式参数：主程序带它启动 = 以守门进程身份执行（内部协议，用户不直接用）。
pub const FENCE_FLAG: &str = "--fence-run";
/// 目的：围栏装不上时守门进程的退出码（工具执行据此如实报错，不静默）。
pub const FENCE_FAILED: i32 = 111;

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

/// 目的：本机能不能强制住这次执行的围栏（机制层自检，**不写本机任何权限项**）。
///   未授权时段靠它把「环境不允许」与「我们写错了」分开——后者绝不能被当成降级吞掉。
pub fn verify(spec: &FenceSpec, command: &str) -> FenceVerdict {
    if selfcheck_forced_unavailable() {
        return FenceVerdict::EnvUnavailable(format!(
            "{}（测试注入：按本机不允许处理）",
            SELFCHECK_FAIL_FLAG
        ));
    }
    backend::verify(spec, command)
}

/// 目的：围栏授权释放的适配器（实现 kernel 共享的 FenceHost 端口）：conductor 只说「这个会话的围栏撤掉」。
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
        #[cfg(windows)]
        {
            windows::release_by_lease(&self.home, lease)
        }
        #[cfg(not(windows))]
        {
            let _ = lease;
            Ok(())
        }
    }

    /// 目的：一次**一次性调用**的收尾——撤它自己的 ACE、删它的按次 profile、删 `private/Packages/<名字>` 空壳。
    /// 约束：三步各自记错、合并返回；profile 删不掉时临时目录仍要尽力清（它只是宿主侧的空壳）。
    fn release_call(&self, spec: &FenceSpec) -> Result<(), String> {
        #[cfg(windows)]
        {
            let mut errors: Vec<String> = Vec::new();
            if let Err(e) = release_fence(spec, &self.home) {
                errors.push(e);
            }
            let name = windows::container_name(spec);
            if let Err(e) = windows::remove_profile_one(&self.home, &name) {
                errors.push(e);
            }
            if let Some(dir) = windows::container_package_dir(spec)
                && dir.is_dir()
                && let Err(e) = std::fs::remove_dir_all(&dir)
            {
                errors.push(format!("删按次临时目录失败（{}）：{}", dir.display(), e));
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("；"))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (spec, &self.home);
            Ok(())
        }
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
    backend::capability()
}

impl Capability {
    /// 目的：本次实际生效的路径级围栏（文件系统, 断网）——本机能力 × 授权。
    ///   只有 Windows 的容器围栏依赖外层写目录 ACL；Linux / macOS 的 Landlock / seatbelt 在守门进程里自足。
    ///   进程树围栏不受授权影响，调用方直接用 `Capability::tree`。
    #[cfg(windows)]
    pub fn effective_paths(&self, write_allowed: bool) -> (bool, bool) {
        if write_allowed {
            (self.fs, self.net)
        } else {
            (false, false)
        }
    }

    /// 目的：同上（非 Windows：没有外层授权这一步，授权值不参与判据）。
    #[cfg(not(windows))]
    pub fn effective_paths(&self, _write_allowed: bool) -> (bool, bool) {
        (self.fs, self.net)
    }
}

pub use job::{FenceJob, launcher, run_fenced};
#[cfg(any(windows, test))]
pub use ledger::Owner;
pub use ledger::{
    Ledger, LedgerEntry, ledger, reconcile, remove_profile_one, restore_one, revoke_grant,
};
#[cfg(windows)]
pub use ledger::{ReconcileReport, release_fence, sweep_orphan_aces};
pub use paths::{fence_env, shell_command};
// 一次围栏准备里真写 ACL 的分步耗时条目：Windows 产生，回执交棒跨平台读。
#[cfg(any(windows, test))]
pub use prep::FencePrep;
pub use prep::FenceStep;
#[cfg(windows)]
pub use prep::prepare_fence;
#[cfg(windows)]
pub use prep::retire_legacy_machine_grants;
pub use prep::{
    MachineGrantReport, clean, ensure_workspace_grant, machine_ensure, machine_missing,
    revoke_machine_grants, sweep_profiles,
};

#[cfg(windows)]
pub(crate) use paths::{
    NODE_REALPATH_FLAGS, container_program_command, view_program_command,
    windows_program_separators,
};
pub(crate) use paths::{interpreter_dir_usable, interpreter_dirs, interpreter_roots_for_exe};
// 视图盘记录清扫要判 pid 活性：复用台账的进程存在性判定。
#[cfg(windows)]
pub(crate) use windows::process_alive;
