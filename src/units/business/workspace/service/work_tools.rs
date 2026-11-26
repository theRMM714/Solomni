//! 目的：共享区版本化工作区的**成员侧执行者**——把十个 work_* 调用接到 workspace 入站面上。
//! 管：成员工具调用的参数解析、路径权限收口与回执转交。
//! 不管：版本库写入、冲突与备份判定（在 workspace）；工具声明（`systools/tools.yaml`）。
//! 联动：`turn` 装配工具面时按 allowed 现取本文件的 `work_handlers`；版本库写入由 workspace 的
//!   service 串行化（它持版本库端口）。

use crate::kernel::api::types::ToolOutcome;
use crate::kernel::ports::ToolHandler;
use crate::units::business::workspace::api as dw;
use crate::units::business::workspace::api::{
    CommitRequest, DiffRequest, ResetRequest, ResolveRequest, ResolveTake, Sandbox, SharedScope,
    Workspace,
};
use std::sync::Arc;

/// 目的：本席的工作区工具执行者：十个 `work_*` 名字由它认领，参数解析与路径权限在这里收口。
pub struct WorkHandler {
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 顶层工作名（共享区与版本库的归属）。
    work: String,
    /// 这一席的 agent 实例名（拉取基线按它记）。
    agent: String,
    /// 这一席的共享区路径归属：拉取与提交的路径在这里收口。
    scope: SharedScope,
}

/// 目的：逐条路径过读权限：拉不进来的东西，读了也没有，所以在这里就拒绝（不作为提交/冲突问题）。
///  纯判定（输入只有共享区归属与路径），便于 T1 直接钉规则。
pub(crate) fn check_readable(scope: &SharedScope, paths: &[String]) -> Result<(), String> {
    if !scope.may_read() {
        return Err("读取被拒：当前会话没有共享区读取权限".to_string());
    }
    for p in paths {
        if !scope.read_ok(p) {
            return Err(format!(
                "读取被拒：{} 不在允许读取的路径内（允许：{}）",
                p,
                scope.read_roots_listing()
            ));
        }
    }
    Ok(())
}

/// 目的：逐条路径过写权限：任一条越界 → 整次提交拒绝（与冲突拒绝同一风格，不静默放过）。
pub(crate) fn check_writable(
    scope: &SharedScope,
    paths: &[String],
    deletes: &[String],
) -> Result<(), String> {
    if !scope.may_commit() {
        return Err("提交被拒：当前会话没有共享区提交权限".to_string());
    }
    for p in paths.iter().chain(deletes.iter()) {
        if !scope.write_ok(p) {
            return Err(format!(
                "提交被拒（越界）：{} 不在允许提交的路径内（允许：{}）",
                p,
                scope.write_roots_listing()
            ));
        }
    }
    Ok(())
}

impl WorkHandler {
    /// 目的：按一席的沙箱事实造执行者（工作名、agent 实例名与共享区归属都从沙箱来）。
    pub fn new(
        workspace: Arc<dyn Workspace + Send + Sync>,
        work: &str,
        agent: &str,
        scope: SharedScope,
    ) -> WorkHandler {
        WorkHandler {
            workspace,
            work: work.to_string(),
            agent: agent.to_string(),
            scope,
        }
    }

    fn sync(&self, session: &str, line: u64, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        if let Err(e) = check_readable(&self.scope, &paths) {
            return outcome(false, e);
        }
        if let Err(e) = check_writable(&self.scope, &paths, &[]) {
            return outcome(false, e);
        }
        // 同步是这一席在当行做的动作：锚落在这里，回档到更早的行才能越过它。
        let anchor = crate::units::business::workspace::api::CommitAnchor {
            agent: self.agent.clone(),
            session: session.to_string(),
            line,
        };
        match self.workspace.work_sync(
            &self.work,
            &paths,
            crate::kernel::api::procs::unix_seconds(),
            anchor,
        ) {
            Ok(r) => outcome(true, dw::render_sync(&r)),
            Err(e) => outcome(false, e),
        }
    }

    fn pull(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        if let Err(e) = check_readable(&self.scope, &paths) {
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
        if let Err(e) = check_writable(&self.scope, &paths, &deletes) {
            return outcome(false, e);
        }
        let message = dw::str_arg(args, "message").unwrap_or_default();
        let req = CommitRequest {
            paths,
            deletes,
            message,
            time: crate::kernel::api::procs::unix_seconds(),
            line,
        };
        match self.workspace.work_commit(&self.work, agent, session, &req) {
            Ok(r) => outcome(true, dw::render_commit(&r)),
            Err(e) => outcome(false, e),
        }
    }

    /// 目的：把已提交的改动推送进主线。
    /// 约束：推送写共享区，所以只过「共享区提交权限」这一道闸；逐条路径的写范围在 work_commit 已经卡过。
    fn push(&self, session: &str, agent: &str, line: u64, args: &serde_json::Value) -> ToolOutcome {
        if !self.scope.may_commit() {
            return outcome(false, "推送被拒：当前会话没有共享区提交权限".to_string());
        }
        let req = dw::PushRequest {
            message: dw::str_arg(args, "message").unwrap_or_default(),
            time: crate::kernel::api::procs::unix_seconds(),
            line,
        };
        match self.workspace.work_push(&self.work, agent, session, &req) {
            Ok(r) => outcome(true, dw::render_push(&r)),
            Err(e) => outcome(false, e),
        }
    }

    /// 目的：比两个版本的行级差异（缺省工作线头 ↔ 沙箱）。
    /// 约束：只读；不在读作用域内的路径不出现（与 work_status 同一收口）。
    fn diff(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        if let Err(e) = check_readable(&self.scope, &paths) {
            return outcome(false, e);
        }
        let from = match commit_id(args, "from") {
            Ok(v) => v,
            Err(e) => return outcome(false, e),
        };
        let to = match commit_id(args, "to") {
            Ok(v) => v,
            Err(e) => return outcome(false, e),
        };
        let req = DiffRequest { from, to, paths };
        match self.workspace.work_diff(&self.work, &self.agent, &req) {
            Ok(mut r) => {
                r.entries.retain(|e| self.scope.read_ok(&e.path));
                outcome(true, dw::render_diff(&r))
            }
            Err(e) => outcome(false, e),
        }
    }

    /// 目的：把沙箱按工作线头恢复（`paths`），或把工作线移回某个提交（`to`）。
    /// 约束：两种形态都只动这一席的沙箱与工作线（沙箱恒可写，不再有权限闸）；
    ///   路径仍要过读作用域（看不见的路径不能借 reset 覆盖）。
    fn reset(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        let to = match commit_id(args, "to") {
            Ok(v) => v,
            Err(e) => return outcome(false, e),
        };
        let keep_changes = match args.get("keep_changes") {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(_) => return outcome(false, "keep_changes 必须是布尔值".to_string()),
        };
        if let Err(e) = check_readable(&self.scope, &paths) {
            return outcome(false, e);
        }
        let req = ResetRequest {
            paths,
            to,
            keep_changes,
        };
        match self.workspace.work_reset(&self.work, &self.agent, &req) {
            Ok(r) => outcome(true, dw::render_reset(&r)),
            Err(e) => outcome(false, e),
        }
    }

    fn status(&self, args: &serde_json::Value) -> ToolOutcome {
        let paths = match dw::str_list(args, "paths") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        if let Err(e) = check_readable(&self.scope, &paths) {
            return outcome(false, e);
        }
        match self.workspace.work_status(&self.work, &self.agent, &paths) {
            Ok(mut r) => {
                // 路径清单也受读作用域约束：不能靠 work_status 看到读不到的路径名。
                r.tree.retain(|p| self.scope.read_ok(p));
                r.entries.retain(|e| self.scope.read_ok(&e.path));
                outcome(true, dw::render_status(&r))
            }
            Err(e) => outcome(false, e),
        }
    }

    /// 目的：读 / 改本工作的过滤名单 `.workignore`（内容面与检索面共用的唯一名单）。
    /// 约束：`list` 只要共享区读权限；`add` / `remove` 改的是「哪些文件算工作区内容」，按共享区提交权限收口。
    fn ignore(&self, args: &serde_json::Value) -> ToolOutcome {
        let op = dw::str_arg(args, "op").unwrap_or_else(|| "list".to_string());
        let patterns = match dw::str_list(args, "patterns") {
            Ok(p) => p,
            Err(e) => return outcome(false, e),
        };
        match op.as_str() {
            "list" => {
                if !self.scope.may_read() {
                    return outcome(false, "读取被拒：当前会话没有共享区读取权限".to_string());
                }
            }
            "add" | "remove" => {
                if !self.scope.may_commit() {
                    return outcome(
                        false,
                        "改过滤规则被拒：当前会话没有共享区提交权限".to_string(),
                    );
                }
            }
            other => {
                return outcome(
                    false,
                    format!("op 只认 list / add / remove，收到：{}", other),
                );
            }
        }
        match self.workspace.work_ignore(&self.work, &op, &patterns) {
            Ok(r) => outcome(true, dw::render_ignore(&r)),
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
                );
            }
        };
        if let Err(e) = check_readable(&self.scope, std::slice::from_ref(&path)) {
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
                );
            }
            None => {
                return outcome(
                    false,
                    "work_resolve 要给出 take（upstream = 取主副本，local = 保留本地）".to_string(),
                );
            }
        };
        if let Err(e) = check_readable(&self.scope, &paths) {
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
            if let Err(e) = check_readable(&self.scope, std::slice::from_ref(&path)) {
                return outcome(false, e);
            }
            return match self
                .workspace
                .work_restore_lost(&self.work, &self.agent, &path, &expect)
            {
                Ok(r) => outcome(true, dw::render_restore_lost(&r)),
                Err(e) => outcome(false, e),
            };
        }
        if let Some(p) = &path
            && let Err(e) = check_readable(&self.scope, std::slice::from_ref(p))
        {
            return outcome(false, e);
        }
        match self
            .workspace
            .work_lost(&self.work, &self.agent, path.as_deref())
        {
            Ok(mut r) => {
                r.entries.retain(|e| self.scope.read_ok(&e.path));
                outcome(true, dw::render_lost(&r))
            }
            Err(e) => outcome(false, e),
        }
    }
}

fn outcome(ok: bool, output: String) -> ToolOutcome {
    ToolOutcome::new(ok, output)
}

/// 目的：从参数里取一个可选提交号（`work_diff` 的 from / to、`work_reset` 的 to）。
fn commit_id(args: &serde_json::Value, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => match n.as_u64() {
            Some(id) => Ok(Some(id)),
            None => Err(format!("{} 必须是非负整数（提交号）", key)),
        },
        Some(_) => Err(format!("{} 必须是非负整数（提交号）", key)),
    }
}

/// 目的：本席要装的工作区工具执行者——席位面没发任何 `work_*` 就一个都不装。
/// 参数：workspace = 共享区用例面；sb = 这一席的沙箱；allowed = 席位面发放的工具 id。
pub fn work_handlers(
    workspace: &Arc<dyn Workspace + Send + Sync>,
    sb: &Sandbox,
    allowed: &[String],
) -> Vec<Arc<dyn ToolHandler>> {
    if !allowed.iter().any(|t| dw::is_work_tool(t)) {
        return Vec::new();
    }
    vec![Arc::new(WorkHandler::new(
        Arc::clone(workspace),
        &sb.work_name,
        &sb.agent,
        sb.scope.clone(),
    ))]
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
            dw::COMMIT => self.commit(ctx.session, ctx.agent, ctx.line, &args),
            dw::PUSH => self.push(ctx.session, ctx.agent, ctx.line, &args),
            dw::SYNC => self.sync(ctx.session, ctx.line, &args),
            dw::STATUS => self.status(&args),
            dw::IGNORE => self.ignore(&args),
            dw::VERSIONS => self.versions(&args),
            dw::DIFF => self.diff(&args),
            dw::RESET => self.reset(&args),
            dw::RESOLVE => self.resolve(&args),
            dw::LOST => self.lost(&args),
            other => outcome(false, format!("未知的工作区工具：{}", other)),
        }
    }
}
