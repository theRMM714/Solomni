//! 目的：工具能力的**状态、用例与端口持有者**——按角色发放工具面、执行工具、释放围栏授权。
//! 管：状态是 `domain` 的 `SystemTools`（工具总表、角色表与名字表）；出站端口是 `SysIo`（本能力私有）
//!   与 kernel 共享的 `ProcessRunner` / `FenceHost` / `FenceGate`——**只有这里持有它们**（R12）。
//! 不管：纯规则（在 `domain/`）；`systools/` 三份 yaml 的加载机制在 `detail/yaml_systools.rs`；本文件不碰文件系统。
//! 联动：对外两个面 `Tools` 与 `ToolExec`（见 `api.rs`）；装配在 `main.rs`。

pub mod fence;
pub mod grep;
pub mod systool;

use self::fence::ToolFenceGate;
use crate::capabilities::tools::api::{
    Observations, Recorded, ToolBook, ToolExec, ToolOutcome, ToolSchema, Tools,
};
use crate::capabilities::tools::domain::budget::{self, Continuation};
use crate::capabilities::tools::domain::roles::SystemTools;
use crate::capabilities::tools::ports::{SysIo, SystoolsSource};
use crate::kernel::ports::{FenceGate, FenceHost, ProcessRunner};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

/// 目的：工具能力——持三张表与四个端口，按用例答话。
pub struct ToolsService {
    /// **状态**：工具总表与角色表。
    catalog: SystemTools,
    runner: Arc<dyn ProcessRunner>,
    io: Arc<dyn SysIo + Send + Sync>,
    fence: Arc<dyn FenceHost + Send + Sync>,
    /// **裁决**：必要落点授不上时怎么处置（问用户 / 无围栏跑一次 / 拒绝）；回执文案归工具层。
    gate: Arc<dyn FenceGate>,
    /// 目的：同一模块工具进程的读写租约（跨 agent）。`parallel` 工具共享读租约，其余取独占写租约。
    module_leases: Mutex<BTreeMap<String, Arc<RwLock<()>>>>,
}

impl ToolsService {
    /// 目的：组合根专用构造——三张表由加载器读进来，三个端口由组合根 `new` 出来注入。
    pub fn new(
        source: &dyn SystoolsSource,
        runner: Arc<dyn ProcessRunner>,
        io: Arc<dyn SysIo + Send + Sync>,
        fence: Arc<dyn FenceHost + Send + Sync>,
    ) -> Result<ToolsService, String> {
        Ok(ToolsService {
            catalog: source.load()?,
            runner,
            io,
            fence,
            gate: Arc::new(ToolFenceGate),
            module_leases: Mutex::new(BTreeMap::new()),
        })
    }

    /// 目的：取（或建）某个模块的进程租约；租约只在同一模块内生效。
    fn module_lease(&self, module: &str) -> Arc<RwLock<()>> {
        let mut leases = self.module_leases.lock().unwrap_or_else(|e| e.into_inner());
        leases
            .entry(module.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }
}

impl Tools for ToolsService {
    fn tool_face(&self, role: &str) -> Result<Vec<(&str, &ToolSchema)>, String> {
        self.catalog.tool_face(role)
    }

    fn role_face(&self, role: &str) -> (Vec<String>, bool) {
        self.catalog.role_face(role)
    }

    fn allows_module_tools(&self, role: &str) -> bool {
        self.catalog.allows_module_tools(role)
    }

    fn roles(&self) -> Vec<String> {
        self.catalog.roles.keys().cloned().collect()
    }

    fn book(&self) -> ToolBook {
        self.catalog.tools.clone()
    }

    fn reserved_agents(&self) -> Vec<String> {
        self.catalog.reserved_agents.clone()
    }

    fn reserved_modules(&self) -> Vec<String> {
        self.catalog.reserved_modules.clone()
    }

    fn problems(&self) -> Vec<String> {
        self.catalog.problems()
    }
}

/// 目的：宿主执行端口的回执（跑没跑、输出是什么、有哪些说明）转成工具面回执。
fn host_outcome(out: crate::kernel::api::run::RunOutcome) -> ToolOutcome {
    ToolOutcome::from(out)
}

/// 所有工具回执送回模型前都过同一个预算（head+tail + 按工具给的续读提示）。
fn within_budget(
    mut out: ToolOutcome,
    hint: Continuation,
    budget: budget::ResultBudget,
) -> ToolOutcome {
    budget::cap_result_with(&mut out, hint, budget);
    out
}

impl ToolExec for ToolsService {
    fn run_module(
        &self,
        module: &str,
        fence: &crate::kernel::api::fence::FencePlan,
        command: &str,
        args_json: &str,
        env: &[(String, String)],
        path: Option<&std::ffi::OsStr>,
        parallel: bool,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
        budget: budget::ResultBudget,
    ) -> ToolOutcome {
        let lease = self.module_lease(module);
        if parallel {
            let _shared = lease.read().unwrap_or_else(|e| e.into_inner());
            within_budget(
                host_outcome(self.runner.run(
                    fence,
                    command,
                    args_json,
                    env,
                    path,
                    ask,
                    Some(self.gate.as_ref()),
                )),
                Continuation::Command,
                budget,
            )
        } else {
            let _exclusive = lease.write().unwrap_or_else(|e| e.into_inner());
            within_budget(
                host_outcome(self.runner.run(
                    fence,
                    command,
                    args_json,
                    env,
                    path,
                    ask,
                    Some(self.gate.as_ref()),
                )),
                Continuation::Command,
                budget,
            )
        }
    }

    fn run_shell(
        &self,
        run: &crate::kernel::ports::ShellRun<'_>,
        budget: budget::ResultBudget,
    ) -> ToolOutcome {
        // 容器授权按会话持有（见 container_name）：调用结束不释放，会话结束/崩溃时统一回收。
        let run = crate::kernel::ports::ShellRun {
            fence: run.fence,
            script: run.script,
            timeout: run.timeout,
            cancel: run.cancel,
            path: run.path,
            ask: run.ask,
            gate: Some(self.gate.as_ref()),
        };
        within_budget(
            host_outcome(self.runner.run_shell(&run)),
            Continuation::Command,
            budget,
        )
    }

    fn run_builtin(
        &self,
        sb: &crate::capabilities::workspace::api::Sandbox,
        builtin_tools: &ToolBook,
        observations: Observations,
        gate: Option<&dyn crate::capabilities::tools::api::WriteGate>,
        budget: budget::ResultBudget,
        name: &str,
        args_json: &str,
    ) -> Recorded {
        let mut observations = observations;
        let outcome = within_budget(
            crate::capabilities::tools::service::systool::execute_with_budget(
                sb,
                builtin_tools,
                self.io.as_ref(),
                &mut observations,
                gate,
                budget,
                name,
                args_json,
            ),
            budget::continuation_for("", name),
            budget,
        );
        Recorded {
            outcome,
            observations,
        }
    }

    fn release_call(&self, spec: &crate::kernel::api::fence::FencePlan) -> Result<(), String> {
        self.fence.release_call(spec)
    }

    fn release_by_lease(&self, lease: &str) -> Result<(), String> {
        self.fence.release_by_lease(lease)
    }
}
