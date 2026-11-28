//! 目的：本机执行环境——不套客机，直接在这一侧跑（策略登记项与执行句柄都在这里）。
//! 管：`HostPolicy`（登记项、参数校验、事实、开句柄）、`HostSandbox`（按端口跑 shell 与模块工具）。
//! 不管：本机侧怎么隔离（内核围栏）；客机怎么跑（`vm`）。
//! 联动：组合根把 `host_policy` 登记进注册表；`agentloop` 只经 `sandbox::api::Sandbox` 调它。

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::kernel::api::types::ToolOutcome;
use crate::kernel::ports::ShellRun;
use crate::units::business::sandbox::api::{
    ExecKind, ExecOutcome, ExecRequest, HOST_POLICY, OpenRequest, PolicyDecl, Sandbox, SandboxDecl,
    SandboxFacts, SandboxPolicy,
};
use crate::units::business::tools::api::ToolExec;

/// 目的：直接造一个本机执行面（不经策略表）：给只建工具面、不建会话的装配与测试用。
pub fn host_sandbox(tools: Arc<dyn ToolExec + Send + Sync>) -> Arc<dyn Sandbox> {
    Arc::new(HostSandbox {
        tools,
        runtime_path: None,
    })
}

/// 目的：本机策略的实例（注册表的第一项，也是没有客机时的缺省）。
pub fn host_policy(tools: Arc<dyn ToolExec + Send + Sync>) -> Arc<dyn SandboxPolicy> {
    Arc::new(HostPolicy { tools })
}

/// 目的：本机执行环境策略（没有客机，也没有参数）。
pub struct HostPolicy {
    tools: Arc<dyn ToolExec + Send + Sync>,
}

impl SandboxPolicy for HostPolicy {
    fn decl(&self) -> PolicyDecl {
        PolicyDecl {
            id: HOST_POLICY.to_string(),
            name: "本机".to_string(),
            hint: "直接在本机跑（不套客机）".to_string(),
            params: None,
            choices: BTreeMap::new(),
        }
    }

    fn check(&self, decl: &SandboxDecl) -> Vec<String> {
        let mut out = Vec::new();
        if !decl.params.is_null() {
            out.push("本机策略没有参数：params 留空".to_string());
        }
        if !decl.pins.is_empty() {
            out.push("本机策略没有定项：pins 留空".to_string());
        }
        out
    }

    fn facts(&self, _decl: &SandboxDecl) -> Result<SandboxFacts, String> {
        Ok(SandboxFacts::host())
    }

    fn open(&self, req: &OpenRequest<'_>) -> Result<Arc<dyn Sandbox>, String> {
        Ok(Arc::new(HostSandbox {
            tools: Arc::clone(&self.tools),
            runtime_path: req.runtime_path.map(Path::to_path_buf),
        }))
    }
}

/// 目的：本机侧的执行句柄——shell 走内核的进程端口，模块工具走工具单元的执行面。
pub struct HostSandbox {
    tools: Arc<dyn ToolExec + Send + Sync>,
    runtime_path: Option<PathBuf>,
}

impl HostSandbox {
    fn runtime(&self) -> Option<&OsStr> {
        self.runtime_path.as_deref().map(Path::as_os_str)
    }
}

impl Sandbox for HostSandbox {
    fn run(&self, req: &ExecRequest<'_>) -> ExecOutcome {
        let outcome: ToolOutcome = match &req.kind {
            ExecKind::Shell { script, .. } => self.tools.run_shell(
                &ShellRun {
                    fence: &req.fence,
                    script,
                    timeout: req.timeout,
                    cancel: req.cancel,
                    path: self.runtime(),
                    ask: req.ask,
                    gate: None,
                },
                req.budget,
            ),
            ExecKind::Module {
                module,
                command,
                args_json,
                env,
                parallel,
            } => self.tools.run_module(
                module,
                &req.fence,
                command,
                args_json,
                env,
                self.runtime(),
                *parallel,
                req.ask,
                req.budget,
            ),
        };
        let prep = crate::kernel::api::process::take_fence_prep().map(|t| t.total);
        ExecOutcome {
            outcome,
            fence_prep: prep,
        }
    }
}
