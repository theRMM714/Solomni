//! 目的：共享区版本化工作区的**成员侧执行者**——把六个 work_* 调用接到 workspace 能力面上。
//! 管：成员工具调用的参数解析、路径权限收口与回执转交。
//! 不管：版本库写入、冲突与备份判定（在 workspace）；工具声明（`systools/tools.yaml`）。
//! 联动：在会话工作线程上执行；成员循环只认"谁认领这个名字"；版本库写入由 workspace 的
//!   service 串行化（它持版本库端口）。

use super::now_ts;
use crate::capabilities::conductor::domain::work as dw;
use crate::capabilities::permission::api::{Decision, PermissionSet, Reach};
use crate::capabilities::workspace::api::{CommitRequest, ResolveRequest, ResolveTake, Workspace};
use crate::kernel::api::ToolOutcome;
use crate::kernel::ports::ToolHandler;
use std::sync::Arc;

pub struct WorkHandler {
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 顶层工作名（共享区与版本库的归属）。
    work: String,
    /// 这一席的 agent 实例名（拉取基线按它记）。
    agent: String,
    /// 这一席的生效权限：提交与拉取的 reach 与共享区范围在这里收口。
    permissions: PermissionSet,
}

/// 逐条路径过读权限：拉不进来的东西，读了也没有，所以在这里就拒绝（不作为提交/冲突问题）。
/// 纯判定（输入只有生效权限与路径），便于 T1 直接钉规则。
pub(crate) fn check_readable(permissions: &PermissionSet, paths: &[String]) -> Result<(), String> {
    if !permissions.reach.contains(Reach::SharedRead) {
        return Err(format!(
            "读取被拒：当前档位「{}」没有共享区读取权限",
            permissions.profile.label()
        ));
    }
    for p in paths {
        if !permissions.read_ok(p) {
            return Err(format!(
                "读取被拒：{} 不在允许读取的路径内（允许：{}）",
                p,
                permissions.read_roots_listing()
            ));
        }
    }
    Ok(())
}

/// 逐条路径过写权限：任一条越界 → 整次提交拒绝（与冲突拒绝同一风格，不静默放过）。
pub(crate) fn check_writable(
    permissions: &PermissionSet,
    paths: &[String],
    deletes: &[String],
) -> Result<(), String> {
    if !permissions.reach.contains(Reach::SharedWrite) {
        return Err(format!(
            "提交被拒：当前档位「{}」没有共享区提交权限",
            permissions.profile.label()
        ));
    }
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
        permissions: PermissionSet,
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
        if let Err(e) = check_readable(&self.permissions, &paths) {
            return outcome(false, e);
        }
        match self.workspace.work_status(&self.work, &self.agent, &paths) {
            Ok(mut r) => {
                // 路径清单也受读作用域约束：不能靠 work_status 看到读不到的路径名。
                r.tree.retain(|p| self.permissions.read_ok(p));
                r.entries.retain(|e| self.permissions.read_ok(&e.path));
                r.local_only.retain(|p| self.permissions.read_ok(p));
                outcome(true, dw::render_status(&r))
            }
            Err(e) => outcome(false, e),
        }
    }

    fn versions(&self, args: &serde_json::Value) -> ToolOutcome {
        let path = match dw::str_arg(args, "path") {
            Some(p) if !p.trim().is_empty() => p,
            _ => {
                return outcome(
                    false,
                    "work_versions 要给出一个文件路径（path）".to_string(),
                )
            }
        };
        if let Err(e) = check_readable(&self.permissions, std::slice::from_ref(&path)) {
            return outcome(false, e);
        }
        match self.workspace.work_versions(&self.work, &self.agent, &path) {
            Ok(v) => outcome(true, dw::render_versions(&v)),
            Err(e) => outcome(false, e),
        }
    }

    fn resolve(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        let expect_head = match args.get("head") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Number(n)) => match n.as_u64() {
                Some(id) => Some(id),
                None => return outcome(false, "head 必须是非负整数（主副本提交 id）".to_string()),
            },
            Some(_) => return outcome(false, "head 必须是非负整数（主副本提交 id）".to_string()),
        };
        let take = match dw::str_arg(args, "take").as_deref() {
            Some("upstream") => ResolveTake::Upstream,
            Some("local") => ResolveTake::Local,
            Some(other) => {
                return outcome(
                    false,
                    format!("take 只认 upstream 或 local，收到：{}", other),
                )
            }
            None => {
                return outcome(
                    false,
                    "work_resolve 要给出 take（upstream = 取主副本，local = 保留本地）".to_string(),
                )
            }
        };
        if let Err(e) = check_readable(&self.permissions, &paths) {
            return outcome(false, e);
        }
        let req = ResolveRequest {
            paths,
            take,
            expect_head,
        };
        match self.workspace.work_resolve(&self.work, &self.agent, &req) {
            Ok(r) => outcome(true, dw::render_resolve(&r)),
            Err(e) => outcome(false, e),
        }
    }

    fn lost(&self, args: &serde_json::Value) -> ToolOutcome {
        let path = dw::str_arg(args, "path");
        let restore = match args.get("restore") {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(_) => return outcome(false, "restore 必须是布尔值".to_string()),
        };
        if restore {
            let Some(path) = path else {
                return outcome(false, "work_lost restore 要给出 path".to_string());
            };
            let Some(expect) = dw::str_arg(args, "expect_current") else {
                return outcome(
                    false,
                    "恢复要带 expect_current（当前沙箱文件的 hash；文件不存在传空串）".to_string(),
                );
            };
            if let Err(e) = check_readable(&self.permissions, std::slice::from_ref(&path)) {
                return outcome(false, e);
            }
            if self.permissions.decisions.write != Decision::Allow
                || !self.permissions.reach.contains(Reach::Private)
            {
                return outcome(
                    false,
                    format!(
                        "恢复被拒：当前档位「{}」不允许写入私有沙箱",
                        self.permissions.profile.label()
                    ),
                );
            }
            return match self
                .workspace
                .work_restore_lost(&self.work, &self.agent, &path, &expect)
            {
                Ok(r) => outcome(true, dw::render_restore_lost(&r)),
                Err(e) => outcome(false, e),
            };
        }
        if let Some(p) = &path {
            if let Err(e) = check_readable(&self.permissions, std::slice::from_ref(p)) {
                return outcome(false, e);
            }
        }
        match self
            .workspace
            .work_lost(&self.work, &self.agent, path.as_deref())
        {
            Ok(mut r) => {
                r.entries.retain(|e| self.permissions.read_ok(&e.path));
                outcome(true, dw::render_lost(&r))
            }
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
            dw::VERSIONS => self.versions(&args),
            dw::RESOLVE => self.resolve(&args),
            dw::LOST => self.lost(&args),
            other => outcome(false, format!("未知的工作区工具：{}", other)),
        }
    }
}
