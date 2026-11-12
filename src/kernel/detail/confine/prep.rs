//! 目的：一次围栏授权的准备与回收（按落点必要性如实收尾）。
//! 管：FencePrep、prepare_fence、profile 清扫、台账回收入口。
//! 不管：台账条目的形状与对账细节（在 ledger.rs）、平台实现（在平台后端）。
//! 联动：由 `confine.rs` 门面与 `process.rs` 的围栏执行消费；Windows 授权细节在 `windows/`。

#[cfg(windows)]
use super::windows;
#[cfg(windows)]
use crate::kernel::api::FenceSpec;
use crate::kernel::api::{FenceBlocked, FencePart};

/// 目的：扫掉本程序建过的整族容器 profile：台账只记"我们知道写过什么"，而 profile 可能来自没有台账的路径
///   （探针、夹具的台账被删、进程被杀）。名字前缀是本程序独有的，所以按它扫。返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    #[cfg(windows)]
    {
        windows::sweep_profiles()
    }
    #[cfg(not(windows))]
    {
        // 其它平台没有容器 profile 这一步。
        Ok(0)
    }
}

/// 目的：一次围栏授权的结论：**必要**落点是否全部授上 + 其余授不上落点的事实。
/// 约束：必要落点授不上时调用方**不许降级**（问用户或按 fail-closed 拒绝，见 docs/kernel/README.md 的「围栏落点：必要与可选」）；
///   可选落点授不上只进 `notes`，不牵动这次执行。
#[derive(Debug, Clone, Default)]
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub struct FencePrep {
    /// 目的：必要落点授不上时的如实结论（`None` = 围栏成立，可以按围栏执行）。
    pub blocked: Option<FenceBlocked>,
    /// 目的：其余授不上落点的事实（可选落点只在这里记一条）。
    pub notes: Vec<String>,
}

impl FencePrep {
    /// 目的：这次授权成不成立（必要落点全授上了）。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn ok(&self) -> bool {
        self.blocked.is_none()
    }

    /// 目的：记一个落点授不上的事实：**必要**落点进 `blocked`（第一次为准——那就是要补的那一环），
    ///   可选落点与后续必要落点进 `notes`。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn fail(&mut self, part: FencePart, path: std::path::PathBuf, why: String) {
        let fact = FenceBlocked { part, path, why };
        if fact.part.necessary() && self.blocked.is_none() {
            self.blocked = Some(fact);
            return;
        }
        self.notes.push(fact.line());
    }
}

/// 目的：外层进程调用：把围栏要用的授权一次性做好（写目录 ACL），并按落点的必要性如实收尾。
///   跳过条件看**实际 ACE**而不是内存台账，所以权限收窄并撤权后能正确重授。
///   只有 Windows 的容器围栏需要这一步——Linux 的 Landlock 与 macOS 的 seatbelt 在守门进程里自足，
///   所以本函数在非 Windows 平台上**不存在**（而不是"存在但空转"）。
#[cfg(windows)]
pub fn prepare_fence(spec: &FenceSpec, command: &str, home: &std::path::Path) -> FencePrep {
    windows::prepare_fence(spec, command, home)
}

/// 目的：精确回收：按台账撤掉我们写过的权限项、删掉我们建过的容器 profile（`--fence-clean` 用）。
pub fn clean(home: &std::path::Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::clean(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Ok("本平台的围栏不留权限项，无需清理".to_string())
    }
}
