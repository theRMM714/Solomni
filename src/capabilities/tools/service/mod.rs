//! 工具能力的**状态、用例与端口持有者**。
//!
//! 状态是 `domain` 的 `SystemTools`（工具总表、角色表与名字表）；出站端口是 `SysIo`（本能力私有）
//! 与 kernel 共享的 `ProcessRunner` / `FenceHost`——**只有这里持有它们**（R12）。
//! 对外两个面：`Tools`（按角色发放工具面、拿总表）与 `ToolExec`（执行工具、释放围栏授权）。
//!
//! 加载机制（`systools/` 三份 yaml）在 `detail/yaml_systools.rs`；本文件不碰文件系统。

pub mod systool;

use crate::capabilities::tools::api::{
    Observations, ToolBook, ToolExec, ToolOutcome, ToolSchema, Tools,
};
use crate::capabilities::tools::domain::roles::SystemTools;
use crate::capabilities::tools::ports::{SysIo, SystoolsSource};
use crate::kernel::ports::{FenceHost, ProcessRunner};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

/// 工具能力：持三张表与三个端口，按用例答话。
pub struct ToolsService {
    /// **状态**：工具总表与角色表。
    catalog: SystemTools,
    runner: Arc<dyn ProcessRunner>,
    io: Arc<dyn SysIo + Send + Sync>,
    fence: Arc<dyn FenceHost + Send + Sync>,
    /// 目的：同一模块工具进程的读写租约（跨 agent）。`parallel` 工具共享读租约，其余取独占写租约。
    module_leases: Mutex<BTreeMap<String, Arc<RwLock<()>>>>,
}

impl ToolsService {
    /// 组合根专用：三张表由加载器读进来，三个端口由组合根 new 出来注入。
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

    fn book(&self) -> ToolBook {
        self.catalog.tools.clone()
    }

    fn reserved_names(&self) -> Vec<String> {
        self.catalog.reserved_names.clone()
    }

    fn problems(&self) -> Vec<String> {
        self.catalog.problems()
    }
}

impl ToolExec for ToolsService {
    fn run_module(
        &self,
        module: &str,
        fence: &crate::kernel::api::FenceSpec,
        command: &str,
        args_json: &str,
        env: &[(String, String)],
        path: Option<&std::ffi::OsStr>,
        parallel: bool,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
    ) -> ToolOutcome {
        let lease = self.module_lease(module);
        if parallel {
            let _shared = lease.read().unwrap_or_else(|e| e.into_inner());
            self.runner.run(fence, command, args_json, env, path, ask)
        } else {
            let _exclusive = lease.write().unwrap_or_else(|e| e.into_inner());
            self.runner.run(fence, command, args_json, env, path, ask)
        }
    }

    fn run_shell(&self, run: &crate::kernel::ports::ShellRun<'_>) -> ToolOutcome {
        // 容器授权按会话持有（见 container_name）：调用结束不释放，会话结束/崩溃时统一回收。
        self.runner.run_shell(run)
    }

    fn run_builtin(
        &self,
        sb: &crate::capabilities::workspace::api::Sandbox,
        builtin_tools: &ToolBook,
        observations: &mut Observations,
        name: &str,
        args_json: &str,
    ) -> ToolOutcome {
        crate::capabilities::tools::service::systool::execute(
            sb,
            builtin_tools,
            self.io.as_ref(),
            observations,
            name,
            args_json,
        )
    }

    fn release_call(&self, spec: &crate::kernel::api::FenceSpec) -> Result<(), String> {
        self.fence.release_call(spec)
    }

    fn release_by_lease(&self, lease: &str) -> Result<(), String> {
        self.fence.release_by_lease(lease)
    }
}
