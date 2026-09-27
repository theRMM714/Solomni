//! 工具能力的**状态、用例与端口持有者**。
//!
//! 状态是 `domain` 的 `SystemTools`（工具总表与角色表）；端口是三个出站端口
//! （`ToolRunner` / `SysIo` / `FenceHost`）——**只有这里持有它们**（R12）。
//! 对外两个面：`Tools`（按角色发放工具面、拿总表）与 `ToolExec`（执行工具、释放围栏授权）。
//!
//! 加载机制（`systools/` 两份 yaml）在 `detail/yaml_systools.rs`；本文件不碰文件系统。

use crate::capabilities::tools::api::{
    Observations, ToolBook, ToolExec, ToolOutcome, ToolSchema, Tools,
};
use crate::capabilities::tools::domain::roles::SystemTools;
use crate::capabilities::tools::ports::{FenceHost, SysIo, SystoolsSource, ToolRunner};
use std::sync::Arc;

/// 工具能力：持两张表与三个端口，按用例答话。
pub struct ToolsService {
    /// **状态**：工具总表与角色表。
    catalog: SystemTools,
    runner: Arc<dyn ToolRunner + Send + Sync>,
    io: Arc<dyn SysIo + Send + Sync>,
    fence: Arc<dyn FenceHost + Send + Sync>,
}

impl ToolsService {
    /// 组合根专用：两张表由加载器读进来，三个端口由组合根 new 出来注入。
    pub fn new(
        source: &dyn SystoolsSource,
        runner: Arc<dyn ToolRunner + Send + Sync>,
        io: Arc<dyn SysIo + Send + Sync>,
        fence: Arc<dyn FenceHost + Send + Sync>,
    ) -> Result<ToolsService, String> {
        Ok(ToolsService {
            catalog: source.load()?,
            runner,
            io,
            fence,
        })
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

    fn problems(&self) -> Vec<String> {
        self.catalog.problems()
    }
}

impl ToolExec for ToolsService {
    fn run_module(
        &self,
        fence: &crate::capabilities::tools::api::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome {
        self.runner.run(fence, command, args_json)
    }

    fn run_builtin(
        &self,
        sb: &crate::capabilities::workspace::api::Sandbox,
        builtin_tools: &ToolBook,
        observations: &mut Observations,
        name: &str,
        args_json: &str,
    ) -> ToolOutcome {
        crate::capabilities::tools::domain::systool::execute(
            sb,
            builtin_tools,
            self.io.as_ref(),
            observations,
            name,
            args_json,
        )
    }

    fn release_fence(
        &self,
        spec: &crate::capabilities::tools::api::FenceSpec,
    ) -> Result<(), String> {
        self.fence.release(spec)
    }
}
