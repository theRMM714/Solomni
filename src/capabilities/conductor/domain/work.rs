//! 目的：共享区版本化工作区三个核心自有工具（`work_pull` / `work_commit` / `work_status`）的纯逻辑——
//! 工具名、入参解析与回执拼装。
//! 管：工具名常量与 `is_work_tool`、取参（`str_list` / `str_arg`）、三份回执的拼装。
//! 不管：真正的拉取 / 提交 / 状态（走 `conductor::service::work_tools` → `workspace::api::Workspace`）；冲突与状态判定（在 `workspace`）。
//! 联动：由本能力的 `service/` 调用；工具名与 `systools/tools.yaml` 同一份名单。

use crate::capabilities::workspace::api::{ChangeKind, CommitReport, PullReport, StatusReport};

pub const PULL: &str = "work_pull";
pub const COMMIT: &str = "work_commit";
pub const STATUS: &str = "work_status";

/// 目的：判定这个名字是不是共享区版本化的工具面（角色表 executor / solo 引用它们）。
pub fn is_work_tool(name: &str) -> bool {
    matches!(name, PULL | COMMIT | STATUS)
}

/// 回执里最多逐个列出的路径数（超出只说数量，不把整棵共享区灌进上下文）。
const MAX_LISTED: usize = 50;

fn paths(list: &[String]) -> String {
    if list.is_empty() {
        return "无".to_string();
    }
    let mut out = list
        .iter()
        .take(MAX_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if list.len() > MAX_LISTED {
        out.push_str(&format!("…（共 {} 条）", list.len()));
    }
    out
}

/// 目的：把 work_pull 的结果说成人能读的几行。
/// 返回：写入 / 跳过 / 冲突 / 主副本没有 各若干行（路径截断规则见 `MAX_LISTED`）。
pub fn render_pull(r: &PullReport) -> String {
    let mut out = format!(
        "已对准主副本提交 {}。写入沙箱 {} 个，已是最新 {} 个，无变化 {} 个，跳过（沙箱已有同名文件）{} 个，冲突 {} 个，主副本没有 {} 个。",
        r.commit,
        r.updated.len(),
        r.already.len(),
        r.no_change.len(),
        r.occupied.len(),
        r.conflicts.len(),
        r.missing.len()
    );
    if !r.updated.is_empty() {
        out.push_str(&format!("\n写入：{}", paths(&r.updated)));
    }
    if !r.occupied.is_empty() {
        out.push_str(&format!(
            "\n跳过（沙箱里已有未被跟踪的同名文件，未覆盖）：{}",
            paths(&r.occupied)
        ));
    }
    if !r.conflicts.is_empty() {
        out.push_str(&format!(
            "\n冲突（本地改了、上游也改了，未覆盖）：{}——先 read 主副本对应路径取上游版本，人工合进沙箱，再 commit。",
            paths(&r.conflicts)
        ));
    }
    if !r.missing.is_empty() {
        out.push_str(&format!("\n主副本没有这些路径：{}", paths(&r.missing)));
    }
    out
}

/// 目的：把 work_commit 的结果说成人能读的几行。
/// 返回：提交 id 与新增 / 修改 / 删除 各若干行。
pub fn render_commit(r: &CommitReport) -> String {
    let mut adds = Vec::new();
    let mut mods = Vec::new();
    let mut dels = Vec::new();
    for c in &r.changes {
        match c.kind {
            ChangeKind::Add => adds.push(c.path.clone()),
            ChangeKind::Modify => mods.push(c.path.clone()),
            ChangeKind::Delete => dels.push(c.path.clone()),
        }
    }
    let mut out = format!(
        "提交 {} 已落盘：新增 {}，修改 {}，删除 {}。",
        r.id,
        adds.len(),
        mods.len(),
        dels.len()
    );
    if !adds.is_empty() {
        out.push_str(&format!("\n新增：{}", paths(&adds)));
    }
    if !mods.is_empty() {
        out.push_str(&format!("\n修改：{}", paths(&mods)));
    }
    if !dels.is_empty() {
        out.push_str(&format!("\n删除：{}", paths(&dels)));
    }
    out
}

/// 目的：把 work_status 的结果说成人能读的几行。
/// 返回：主副本当前提交与路径、你的工作副本状态、最近提交。
pub fn render_status(r: &StatusReport) -> String {
    let mut out = String::new();
    match &r.head {
        Some(h) => out.push_str(&format!(
            "主副本当前提交 {}（{}，{}）：{}",
            h.id, h.author, h.time, h.message
        )),
        None => out.push_str("主副本还没有任何提交"),
    }
    if !r.tree.is_empty() {
        out.push_str(&format!("\n主副本路径：{}", paths(&r.tree)));
        if r.tree_truncated {
            out.push_str("…（已截断，只列了一部分）");
        }
    }
    if !r.entries.is_empty() {
        out.push_str("\n你的工作副本状态：");
        for e in &r.entries {
            out.push_str(&format!("\n- {}：{}", e.path, e.state.as_str()));
        }
    }
    if !r.recent.is_empty() {
        out.push_str("\n最近提交：");
        for c in &r.recent {
            out.push_str(&format!("\n- {} {}：{}", c.id, c.author, c.message));
        }
    }
    out
}

/// 目的：从 args 里取一个字符串数组（缺省 = 空）。
/// 错误：不是数组、或数组里有非字符串时如实报错。
pub fn str_list(args: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::new();
            for it in items {
                match it.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(format!("{} 的每一项都必须是字符串", key)),
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("{} 必须是一个字符串数组", key)),
    }
}

/// 目的：从 args 里取一个字符串（缺省 = `None`）。
pub fn str_arg(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
