//! 共享区版本化工作区的**成员侧执行者**：把 `work_pull` / `work_commit` / `work_status` 三个调用
//! 接到 `workspace` 能力面上（与代理工具同理：成员循环只认"谁认领这个名字"，核心自有工具不是特例）。
//!
//! 执行发生在会话工作线程上；版本库写入由 workspace 的 service 串行化（它持版本库端口）。

use super::now_ts;
use crate::capabilities::conductor::domain::work as dw;
use crate::capabilities::permission::api::Permissions;
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
    /// 这一席的生效权限：**提交与拉取的作用域**在这里收口（白名单/黑名单只认一个正向判定）。
    permissions: Permissions,
}

/// 逐条路径过读权限：拉不进来的东西，读了也没有，所以在这里就拒绝（不作为提交/冲突问题）。
/// 纯判定（输入只有生效权限与路径），便于 T1 直接钉规则。
pub(crate) fn check_readable(permissions: &Permissions, paths: &[String]) -> Result<(), String> {
    for p in paths {
        if !permissions.read_ok(p) {
            return Err(format!(
                "拉取被拒：{} 不在允许读取的路径内（允许：{}）",
                p,
                permissions.read_roots_listing()
            ));
        }
    }
    Ok(())
}

/// 逐条路径过写权限：任一条越界 → 整次提交拒绝（与冲突拒绝同一风格，不静默放过）。
pub(crate) fn check_writable(
    permissions: &Permissions,
    paths: &[String],
    deletes: &[String],
) -> Result<(), String> {
    for p in paths.iter().chain(deletes.iter()) {
        if !permissions.write_ok(p) {
            return Err(format!(
                "提交被拒（越界）：{} 不在允许提交的路径内（允许：{}）",
                p,
                permissions.write_roots_listing()
            ));
        }
    }
    Ok(())
}

impl WorkHandler {
    pub fn new(
        workspace: Arc<dyn Workspace + Send + Sync>,
        work: &str,
        agent: &str,
        permissions: Permissions,
    ) -> WorkHandler {
        WorkHandler {
            workspace,
            work: work.to_string(),
            agent: agent.to_string(),
            permissions,
        }
    }

    fn pull(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        if let Err(e) = check_readable(&self.permissions, &paths) {
            return outcome(false, e);
        }
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
        if let Err(e) = check_writable(&self.permissions, &paths, &deletes) {
            return outcome(false, e);
        }
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
