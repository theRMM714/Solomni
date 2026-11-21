//! 目的：一次围栏授权的准备与回收（按落点必要性如实收尾）。
//! 管：FencePrep、prepare_fence、profile 清扫、台账回收入口。
//! 不管：台账条目的形状与对账细节（在 ledger.rs）、平台实现（在平台后端）。
//! 联动：由 `confine.rs` 门面与 `process.rs` 的围栏执行消费；Windows 授权细节在 `windows/`。

use crate::kernel::contracts::fence::FenceBlocked;
use crate::kernel::contracts::fence::FencePart;

#[cfg(windows)]
mod windows;
#[cfg(all(test, windows))]
pub use windows::clean;
#[cfg(windows)]
pub use windows::prepare_fence;

/// 目的：扫掉本程序建过的整族容器 profile：台账只记"我们知道写过什么"，而 profile 可能来自没有台账的路径
///   （探针、夹具的台账被删、进程被杀）。名字前缀是本程序独有的，所以按它扫。返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    super::admin::sweep_profiles()
}

/// 目的：一次围栏准备里**真写了 ACL** 的一个步骤：脱敏标签 + 耗时（毫秒）。
/// 约束：标签只用固定词表（解释器目录 / 祖先链 / 受保护根 / 可信基座），不写用户绝对路径；
///   纯命中已有 ACE、跳过不写的落点不产生步骤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceStep {
    /// 目的：步骤类别（固定词表，见 `windows.rs` 的 STEP_* 常量）。
    pub label: String,
    /// 目的：这一步真写 ACL 的墙钟耗时（毫秒）。
    pub ms: u64,
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
    /// 目的：这次真写 ACL 的分步耗时（纯命中 / 没写时为空，回执与 stderr 只用它判断"钱花在哪"）。
    pub steps: Vec<FenceStep>,
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

/// 目的：一次性的**机器级**预授还缺哪些落点（空 = 已就绪）。只读、O(深度 + 2)，启动对账用。
///   Windows 之外没有「机器级 ACL」这一步，如实返回未支持而不是假称就绪。
pub fn machine_missing(root: &std::path::Path) -> Result<Vec<String>, String> {
    super::admin::machine_missing(root)
}

/// 目的：把机器级落点一次性授好（幂等；每条进度交给 `out`）。`--setup=fence` 与首次启动授权共用。
pub fn machine_ensure(
    root: &std::path::Path,
    home: &std::path::Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    super::admin::machine_ensure(root, home, out)
}

/// 目的：退役台账里旧的全应用包**机器级**授权（`--setup=fence` 顺带做的一次性外部状态清理）。
///   先授后撤：能力 SID 的 ACE 没覆盖那条权限位就保留；系统目录自带的 ACE 不在台账里，一条都不碰。
pub fn retire_legacy_machine_grants(
    home: &std::path::Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    super::admin::retire_legacy_machine_grants(home, out)
}

/// 目的：全量回收：连机器级授权一起撤（`--fence-clean` 用）。
pub fn clean_all(home: &std::path::Path) -> Result<String, String> {
    super::admin::clean_all(home)
}
