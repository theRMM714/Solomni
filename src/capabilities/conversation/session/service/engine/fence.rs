//! 目的：成员回合的工具围栏——写入门禁的发布口（work_commit）与不透明写者的回合边界采纳。
//! 管：发布前按观察账本拒绝未读的已追踪路径；shell / 模块工具落地后采纳沙箱改动并给出有界回执。
//! 不管：门禁本体（在 tools::api）；工具调度（在 engine::tools）；读写盘（在 tools 端口后）。
//! 联动：调用方是 engine::tools 的 run_branch / run_batch；观察事实形状见 tools::api::ObservationFact。

use crate::capabilities::conversation::session::domain::tools::MemberTools;
use crate::capabilities::tools::api::{ObservationFact, SessionWriteGate, ShellFacts, WriteGate};
use crate::kernel::api::types::ToolOutcome;

/// 目的：一次调用的结果形态（标签，结果，观察事实，shell 结构化事实），只为把签名读短。
pub(crate) type CallDone = (
    String,
    ToolOutcome,
    Vec<ObservationFact>,
    Option<ShellFacts>,
);

/// 目的：work_commit 的发布口准入——只拦"已追踪、且当前内容与基线不同"的路径。
/// 约束：未追踪路径不拦（与写工具同一 tracked 谓词）；显式删除不要求读过；没有工作区（直调测试）不拦。
pub(super) fn commit_refusal(
    ctx: &MemberTools,
    name: &str,
    args_json: &str,
    facts: &[crate::capabilities::tools::api::ObservationFact],
) -> Option<ToolOutcome> {
    if name != "work_commit" {
        return None;
    }
    let ws = ctx.workspace.as_ref()?;
    let args: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let strings = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut paths = strings("paths");
    let deletes = strings("deletes");
    if paths.is_empty() {
        let report = ws
            .work_status(&ctx.face.sandbox.work_name, &ctx.face.sandbox.agent, &[])
            .ok()?;
        paths = report.entries.into_iter().map(|e| e.path).collect();
    }
    paths.retain(|p| !deletes.contains(p));
    if paths.is_empty() {
        return None;
    }
    let infos =
        match ws.work_gate_info(&ctx.face.sandbox.work_name, &ctx.face.sandbox.agent, &paths) {
            Ok(v) => v,
            Err(e) => return Some(ToolOutcome::new(false, e)),
        };
    let gate = SessionWriteGate {
        workspace: ws.as_ref(),
        work: &ctx.face.sandbox.work_name,
        agent: &ctx.face.sandbox.agent,
        facts,
        worktree: &ctx.face.sandbox.agent,
        log: Some(ctx.log.as_ref()),
    };
    for info in &infos {
        if !info.tracked {
            continue;
        }
        let changed = match (&info.current, &info.base) {
            (None, None) => false,
            (Some(c), Some(b)) => c.hash != b.hash,
            _ => true,
        };
        if !changed {
            continue;
        }
        if let Err(f) = gate.check(
            &info.path,
            crate::capabilities::tools::api::Need::Observed,
            info.current.as_ref(),
        ) {
            return Some(ToolOutcome::new(
                false,
                format!(
                    "work_commit 不能提交未读的已追踪路径 {}：{}",
                    info.path,
                    f.message(&info.path)
                ),
            ));
        }
    }
    None
}
/// 目的：把 shell / 模块工具落进沙箱的改动，在**回合边界**采纳成共享区提交
///   （write / edit / patch 的改动已在各自调用后由 commit_writes 提交）。
/// 约束：只有这一批里真的跑成过不透明工具（shell / 模块工具）才扫全盘；工作区缺失、外部工作区、
///   采纳失败都不打断回合——回执挂在这一批最后一个跑成的不透明工具结果后面。
pub(super) fn adopt_sandbox(
    ctx: &MemberTools,
    plan: &[(Option<String>, String, String)],
    done: &mut [Option<CallDone>],
    facts: &[crate::capabilities::tools::api::ObservationFact],
) {
    let Some(ws) = ctx.workspace.as_ref() else {
        return;
    };
    if ctx.face.sandbox.work_name.is_empty() {
        return;
    }
    // 最后一个跑成的不透明工具：回执挂它的结果后面（下一次生成能看到的通道）。
    let mut last: Option<usize> = None;
    for (i, (module, tool, _)) in plan.iter().enumerate() {
        if module.is_none() && !crate::capabilities::tools::api::is_exec(tool) {
            continue;
        }
        if matches!(&done[i], Some((_, o, _, _)) if o.ok) {
            last = Some(i);
        }
    }
    let Some(i) = last else {
        return;
    };
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let line = ctx.line.load(std::sync::atomic::Ordering::Relaxed);
    let note = match ws.work_adopt_sandbox(
        &ctx.face.sandbox.work_name,
        &ctx.face.sandbox.agent,
        &ctx.face.sandbox.session,
        time,
        line,
    ) {
        Ok(report) => adopt_note(ctx, &report, facts),
        Err(e) => format!("\n\n[沙箱改动未存档] 这些文件没有记进存档：{}", e),
    };
    if let Some((_, outcome, _, _)) = &mut done[i] {
        outcome.output.push_str(&note);
    }
}

/// 目的：沙箱采纳的回执（给模型看）；没有改动也没有没采纳的路径时为空串。
/// 约束：逐路径标出"当前版本看过没有"，并附**有界** diff 摘要——回执只是提醒，不代替 read。
fn adopt_note(
    ctx: &MemberTools,
    report: &crate::capabilities::workspace::api::AdoptReport,
    facts: &[crate::capabilities::tools::api::ObservationFact],
) -> String {
    const MAX_PATHS: usize = 8;
    const MAX_LINES: usize = 40;
    const MAX_BYTES: usize = 8 * 1024;
    let mut out = String::new();
    if let Some(id) = report.commit {
        out.push_str(&format!(
            "\n\n[沙箱改动已存档 {}] 这些文件不是 write / edit 改的（shell 或模块工具），已记进你的存档线。",
            id
        ));
    }
    let Some(ws) = ctx.workspace.as_ref() else {
        if !report.changed.is_empty() {
            out.push_str(&format!("\n改动路径：{}", report.changed.join("、")));
        }
        for h in &report.held {
            out.push_str(&format!("\n[未采纳] {}：{}", h.path, h.why));
        }
        return out;
    };
    let work = &ctx.face.sandbox.work_name;
    let agent = &ctx.face.sandbox.agent;
    let infos = ws
        .work_gate_info(work, agent, &report.changed)
        .unwrap_or_default();
    let mut unread: Vec<String> = Vec::new();
    if !report.changed.is_empty() {
        out.push_str("\n改动路径（当前版本）：");
        for path in report.changed.iter().take(MAX_PATHS) {
            let version = infos
                .iter()
                .find(|i| &i.path == path)
                .and_then(|i| i.current.as_ref().or(i.base.as_ref()))
                .map(|f| f.hash.as_str());
            let seen = version.is_some_and(|v| {
                facts
                    .iter()
                    .any(|f| f.path == *path && f.worktree == *agent && f.version == v)
            });
            out.push_str(&format!(
                "\n  - {}：{}",
                path,
                if seen { "已读" } else { "未读" }
            ));
            if !seen {
                unread.push(path.clone());
            }
        }
        if report.changed.len() > MAX_PATHS {
            out.push_str(&format!(
                "\n  ……还有 {} 个路径未列出（先 read 再改）",
                report.changed.len() - MAX_PATHS
            ));
        }
        if !unread.is_empty() {
            out.push_str(
                "\n这些路径的当前内容尚未被你看过；直接 edit / write 会被门禁拒绝，先 read。",
            );
        }
    }
    for h in &report.held {
        out.push_str(&format!("\n[未采纳] {}：{}", h.path, h.why));
    }
    // 有界 diff：最多前 8 个路径、40 行、8 KiB；超出只报增删行数与"已截断"。
    let paths: Vec<String> = report.changed.iter().take(MAX_PATHS).cloned().collect();
    if paths.is_empty() {
        return out;
    }
    let Ok(diff) = ws.work_diff(
        work,
        agent,
        &crate::capabilities::workspace::api::DiffRequest {
            from: None,
            to: None,
            paths,
        },
    ) else {
        return out;
    };
    let mut shown = 0usize;
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut body = String::new();
    for e in &diff.entries {
        let mut lines: Vec<String> = Vec::new();
        if e.binary {
            lines.push("（二进制文件，不展开行差异）".to_string());
        } else if e.too_large {
            lines.push("（超过单文件上限，不展开行差异；用 read 看当前内容）".to_string());
        } else {
            for h in &e.hunks {
                for line in &h.lines {
                    match line.sign {
                        crate::capabilities::workspace::api::DiffSign::Add => added += 1,
                        crate::capabilities::workspace::api::DiffSign::Remove => removed += 1,
                        crate::capabilities::workspace::api::DiffSign::Context => {}
                    }
                    if shown < MAX_LINES && body.len() < MAX_BYTES {
                        let sign = match line.sign {
                            crate::capabilities::workspace::api::DiffSign::Add => '+',
                            crate::capabilities::workspace::api::DiffSign::Remove => '-',
                            crate::capabilities::workspace::api::DiffSign::Context => ' ',
                        };
                        lines.push(format!("{}{}", sign, line.text));
                        shown += 1;
                    }
                }
            }
        }
        if lines.is_empty() {
            continue;
        }
        body.push_str(&format!("\n--- a/{}\n+++ b/{}", e.path, e.path));
        for line in lines {
            body.push('\n');
            body.push_str(&line);
        }
    }
    if added + removed == 0 {
        return out;
    }
    out.push_str("\n[沙箱改动摘要，已截断]");
    out.push_str(&body);
    let total = added + removed;
    if total > shown || shown >= MAX_LINES || body.len() >= MAX_BYTES {
        out.push_str(&format!(
            "\n……还有约 {} 行未显示（+{} / -{}）",
            total.saturating_sub(shown),
            added,
            removed
        ));
    }
    out
}
