//! 目的：kernel 自己的纯机制——按次租约与全局沙箱要求；不含围栏描述符，也不含裁决与文案。
//! 管：`call_lease`、`SandboxRequirements`、`SandboxMode`。
//! 不管：围栏描述符（在 `kernel/contracts/fence.rs`）；从 `Sandbox` / 模块权限**派生**围栏
//!   （在 `capabilities/tools/domain/fence.rs`）；平台机制（Landlock / seatbelt / AppContainer）在 `detail/confine`。
//! 联动：由 `src/kernel/api.rs` 重导出；启动期能力探测与工具执行层消费它。

use crate::kernel::contracts::fence::FenceSpec;

/// 目的：给**一次性调用**（构建 / 独立模块工具）生成唯一围栏租约；会话内的调用直接用会话 id 当租约。
/// 约束：同一进程内单调递增 + pid；会话名只作可读前缀，不参与唯一性。
pub fn call_lease(session: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{}#call-{}-{}", session, std::process::id(), n)
}

/// 目的：全局沙箱策略对机制的要求（与 `settings.yaml` 的 `sandbox.policy.enforce` 同义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SandboxRequirements {
    pub filesystem: bool,
    pub network: bool,
    pub process_tree: bool,
    pub env_whitelist: bool,
}

/// 目的：本进程的沙箱模式——全局要求 + 本机能力 + 是否已同意本进程降级。
/// 约束：降级只活在内存里，不落盘；重启后重新评估、重新询问。
#[derive(Debug)]
pub struct SandboxMode {
    pub requirements: SandboxRequirements,
    pub filesystem_ok: bool,
    pub network_ok: bool,
    pub tree_ok: bool,
    pub env_ok: bool,
    /// 目的：`on_unavailable=ask` 时为真；`refuse` 时为假（不允许通过问答降级）。
    pub ask_allowed: bool,
    degraded: std::sync::atomic::AtomicBool,
}

impl SandboxMode {
    /// 目的：按全局要求与本机能力构造；初始未降级。
    pub fn new(
        requirements: SandboxRequirements,
        filesystem_ok: bool,
        network_ok: bool,
        tree_ok: bool,
        env_ok: bool,
        ask_allowed: bool,
    ) -> SandboxMode {
        SandboxMode {
            requirements,
            filesystem_ok,
            network_ok,
            tree_ok,
            env_ok,
            ask_allowed,
            degraded: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 目的：测试替身/无策略场景——什么都不要求，机制视为可用。
    pub fn permissive() -> SandboxMode {
        SandboxMode::new(
            SandboxRequirements::default(),
            true,
            true,
            true,
            true,
            false,
        )
    }

    pub fn degraded(&self) -> bool {
        self.degraded.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_degraded(&self, value: bool) {
        self.degraded
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }

    /// 目的：启动期缺哪些必要机制（不看 `prepared`，那是执行期事实）。
    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.requirements.filesystem && !self.filesystem_ok {
            out.push("文件系统围栏");
        }
        if self.requirements.network && !self.network_ok {
            out.push("网络围栏");
        }
        if self.requirements.process_tree && !self.tree_ok {
            out.push("进程树围栏");
        }
        if self.requirements.env_whitelist && !self.env_ok {
            out.push("环境白名单");
        }
        out
    }

    /// 目的：执行前检查——要求强制但机制（或 Windows 的本机授权）不可用时拒绝；已同意降级则放行。
    /// 约束：`spec.net=true` 时不需要“网络围栏”来拦出站；只有要求断网时才检查。
    pub fn check(&self, spec: &FenceSpec, prepared: bool) -> Result<(), String> {
        let paths_effective = !crate::kernel::api::HAS_EXTERNAL_ACL_STEP || prepared;
        let mut missing: Vec<&'static str> = Vec::new();
        if self.requirements.filesystem && (!self.filesystem_ok || !paths_effective) {
            missing.push("文件系统围栏");
        }
        if self.requirements.network && !spec.net && (!self.network_ok || !paths_effective) {
            missing.push("网络围栏");
        }
        if self.requirements.process_tree && !self.tree_ok {
            missing.push("进程树围栏");
        }
        if self.requirements.env_whitelist && !self.env_ok {
            missing.push("环境白名单");
        }
        if missing.is_empty() || self.degraded() {
            return Ok(());
        }
        Err(format!(
            "沙箱要求{}，但本机机制不可用或未获授权；按全局策略拒绝执行（启动时可选择本进程降级，重启后重新询问）",
            missing.join("、")
        ))
    }
}
