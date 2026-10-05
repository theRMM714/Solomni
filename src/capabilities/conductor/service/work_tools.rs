//! 共享区版本化工作区的**成员侧执行者**：把 `work_pull` / `work_commit` / `work_status` 三个调用
//! 接到 `workspace` 能力面上（与代理工具同理：成员循环只认"谁认领这个名字"，核心自有工具不是特例）。
//!
//! 执行发生在会话工作线程上；版本库写入由 workspace 的 service 串行化（它持版本库端口）。

use super::now_ts;
use crate::capabilities::conductor::domain::work as dw;
use crate::capabilities::workspace::api::{CommitRequest, Workspace};
use crate::kernel::api::ToolOutcome;
use crate::kernel::ports::ToolHandler;
use std::sync::Arc;

pub struct WorkHandler {
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 顶层工作名（共享区与版本库的归属）。
    work: String,
    /// 这一席的 agent 实例名（拉取基线按它记）。
    agent: String,
}

impl WorkHandler {
    pub fn new(
        workspace: Arc<dyn Workspace + Send + Sync>,
        work: &str,
        agent: &str,
    ) -> WorkHandler {
        WorkHandler {
            workspace,
            work: work.to_string(),
            agent: agent.to_string(),
        }
    }

    fn pull(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        match self.workspace.work_pull(&self.work, &self.agent, &paths) {
            Ok(r) => outcome(true, dw::render_pull(&r)),
            Err(e) => outcome(false, e),
        }
    }

    fn commit(
        &self,
        session: &str,
        agent: &str,
        line: u64,
        args: &serde_json::Value,
    ) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        let deletes = match dw::str_list(args, "deletes") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        let message = dw::str_arg(args, "message").unwrap_or_default();
        let req = CommitRequest {
            paths,
            deletes,
            message,
            time: now_ts(),
            line,
        };
        match self.workspace.work_commit(&self.work, agent, session, &req) {
            Ok(r) => outcome(true, dw::render_commit(&r)),
            Err(e) => outcome(false, e),
        }
    }

    fn status(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        match self.workspace.work_status(&self.work, &self.agent, &paths) {
            Ok(r) => outcome(true, dw::render_status(&r)),
            Err(e) => outcome(false, e),
        }
    }
}

fn outcome(ok: bool, output: String) -> ToolOutcome {
    ToolOutcome { ok, output }
}

impl ToolHandler for WorkHandler {
    fn owns(&self, name: &str) -> bool {
        dw::is_work_tool(name)
    }

    fn run(&self, ctx: &crate::kernel::ports::ToolCtx, name: &str, args_json: &str) -> ToolOutcome {
        let args: serde_json::Value = match serde_json::from_str(args_json) {
            Ok(v) => v,
            Err(e) => return outcome(false, format!("参数不是合法 JSON：{}", e)),
        };
        match name {
            dw::PULL => self.pull(&args),
            dw::COMMIT => self.commit(ctx.work, ctx.agent, ctx.line, &args),
            dw::STATUS => self.status(&args),
            other => outcome(false, format!("未知的工作区工具：{}", other)),
        }
    }
}
