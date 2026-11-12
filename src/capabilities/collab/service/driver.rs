//! 目的：回合驱动——在一条 AgentSession 上跑一个回合（单 agent 的话 / 子会话的任务 / 讨论席的发言 / 压缩）。
//! 管：say / dispatch_task / discussion_turn / continue_reply / compact_session 五个自由函数，把会话当参数驱动。
//! 不管：会话类型（归会话能力）与轮循环本体（在 round.rs）；不给别人的类型写 impl（会形成互相引入）。
//! 联动：依赖 crate::capabilities::session::api::AgentSession 与 round::converse_with；契约见 docs/session/session-model.md。

use super::discussion::*;
use super::round::*;
use crate::capabilities::llm::api::{Chunk, Msg};
use crate::capabilities::session::api::{
    delta_visible, stream_piece, AgentSession, LineView, Live, SessionEvent, ToolCallView, TurnRun,
};
/// 目的：只把被总结的前缀发给模型、只声明 compact 工具，跑一次模型并返回摘要。
/// 约束：两条通道都认：native 从结构化槽位取，信封通道从正文里的信封取（与讨论回合同口径）。
pub fn compact_turn(
    s: &mut AgentSession,
    prompt: &str,
    plan: &crate::capabilities::session::api::CompactionPlan,
    identity: &str,
) -> Result<String, String> {
    let decl = s
        .tools
        .as_ref()
        .and_then(|t| t.builtin_tools.get("compact"))
        .map(|t| t.decl("compact"));
    let mut spec = s.compact_chat()?;
    let thinking = if spec.is_some() { None } else { s.thinking() };
    let cut = plan.cut.min(s.dialogue.len());
    // 整条消息**只有这一处装配**：身份 + 本回合工具（只有 compact）+ 被总结前缀 + 压缩提示。
    let msgs = crate::capabilities::collab::service::round::assemble(
        identity,
        s.tools.as_ref(),
        &["compact".to_string()],
        false,
        &s.dialogue[..cut],
        &[Msg::user(prompt.to_string())],
    );
    if let Some(c) = spec.as_ref() {
        let chars: usize = msgs.iter().map(|m| m.content.chars().count()).sum();
        let estimate = (chars / 4).max(if chars > 0 { 1 } else { 0 }) as u64;
        if c.window > 0 && estimate > c.window {
            return Err(format!(
                "压缩模型 {} 的上下文窗口不足：被总结前缀约 {} tokens，窗口 {}；换更大窗口的模型或提高保留比例",
                c.model, estimate, c.window
            ));
        }
    }
    let mut opts =
        crate::capabilities::llm::api::CompleteOpts::plain(false).with_thinking(thinking);
    if let Some(d) = decl.as_ref() {
        opts.tools = Some(std::slice::from_ref(d));
    }
    let mut keep = |_c: crate::capabilities::llm::api::Chunk| true;
    let chat: &mut crate::capabilities::llm::api::BoxedChat = match spec.as_mut() {
        Some(c) => &mut c.chat,
        None => &mut s.chat,
    };
    let done = chat.complete(&msgs, opts, &mut keep);
    if let Some(err) = done.error {
        return Err(err);
    }
    let (name, args) = match done.calls.first() {
        Some(c) => (c.name.clone(), c.args_json.clone()),
        None => {
            let r = crate::capabilities::llm::api::parse(&done.raw);
            let t = r
                .tools
                .first()
                .ok_or_else(|| "压缩回合没有调用 compact（没给出摘要）".to_string())?;
            (t.name.clone(), t.args_json.clone())
        }
    };
    if name != "compact" {
        return Err(format!("压缩回合该只调 compact，实际调了 {}", name));
    }
    let v: serde_json::Value =
        serde_json::from_str(&args).map_err(|e| format!("压缩参数不合法（{}）：{}", e, args))?;
    let summary = v
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    if summary.trim().is_empty() {
        return Err("压缩回合给出的摘要是空的".to_string());
    }
    Ok(summary)
}

/// 目的：一次压缩的完整结果（记录行 + 用量）。
/// 约束：调用方负责推事件 / 落盘。
pub struct CompactedTurn {
    /// 目的：压缩记录行（摘要 + up_to）。
    pub line: LineView,
    /// 目的：压完后的发送视图估算用量。
    pub usage: crate::capabilities::session::api::ContextUsage,
}

/// 目的：压一次：算切分点 → 只把被总结前缀交给模型 → 应用并落压缩记录行。
/// 返回：没有可压缩内容时 `None`（调用方决定提示还是静默跳过）。
/// 错误：模型调用失败 / 摘要为空时如实返回原因。
pub fn compact_session(
    s: &mut AgentSession,
    identity: &str,
    protect_from: Option<u64>,
) -> Result<Option<CompactedTurn>, String> {
    let Some(plan) = s.plan_compaction(protect_from) else {
        return Ok(None);
    };
    let prompt = s.compact_prompt.clone();
    let summary = compact_turn(s, &prompt, &plan, identity)?;
    let Some(line) = s.apply_compaction(&plan, &summary) else {
        return Ok(None);
    };
    let usage = s.estimate_usage();
    s.set_last_usage(Some(usage));
    Ok(Some(CompactedTurn { line, usage }))
}

/// 到点自动压一次：有供应商 usage 用精确 token；拿不到才按字符估算（≈ tokens × 4）。
/// 压不动就**如实通知并继续用完整上下文**（不静默降级、不假装压过）。
fn maybe_compact(
    s: &mut AgentSession,
    identity: &str,
    protect_from: Option<u64>,
    sink: &mut dyn FnMut(SessionEvent),
) {
    if s.compact_percent == 0 {
        return;
    }
    let exceeded = match s.context_usage() {
        // 供应商报过 usage：用它和窗口比，别让字符估算与状态栏的百分比各说各话。
        Some(u) if u.window > 0 => u.total_tokens * 100 >= u.window * s.compact_percent as u64,
        // 没报过：退回字符预算。
        _ => {
            if s.compact_at == 0 {
                return;
            }
            s.dialogue_chars() > s.compact_at
        }
    };
    if !exceeded {
        return;
    }
    match compact_session(s, identity, protect_from) {
        Ok(Some(c)) => {
            sink(SessionEvent::Transcript(vec![c.line.clone()]));
            // 压完发送视图变小了：留一份估算用量，别让下一轮继续拿旧的大数字反复触发。
            sink(SessionEvent::Usage {
                usage: c.usage,
                line: c.line.id,
            });
        }
        Ok(None) => {}
        Err(err) => {
            let note = format!("[警告] 自动压缩没成功：{}（继续用完整上下文）", err);
            for ev in s.note_notice(&note) {
                sink(ev);
            }
        }
    }
}

/// 目的：发言：先把 @ 引用改写成寻址 → 压入用户消息 → 逐轮（文本行 / 工具行）落转录。
///   改写在这一处完成，所以转录行与进上下文的消息是同一份文本（转录即内容）。
pub fn say(
    s: &mut AgentSession,
    text: &str,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
    let text = crate::capabilities::prompt::api::rewrite(text, Some(&s.id), &s.roots());
    s.dialogue.push(Msg::user(text.clone()));
    // 用户行不属于任何模型回复：给它**自己的行号**当回复号（与重建时的规则一致），
    // 否则它会继承上一轮的回复号，回档时与上一轮误并成一组。
    s.cur_reply = s.next_line;
    let user_line = s.line(text.to_string(), "user", "用户", "", None, None);
    let user_line_id = user_line.id;
    sink(SessionEvent::Transcript(vec![user_line]));
    // 到点先压一次：本轮刚收到的用户消息必须原样保留，更早的内容才进摘要。
    maybe_compact(s, identity, Some(user_line_id), sink);
    rounds_events(s, identity, live, sink);
}

/// 目的：**派发并跑这一回合**：注入任务（`note_task`）后正常问模型。
///   节点派发只有这一条语义——CLI 与 Web 各自只是"点火"，不各写一套（见 session-model.md 四之二）。
pub fn dispatch_task(
    s: &mut AgentSession,
    text: &str,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
    let mut task_line = 0u64;
    for e in s.note_task(text) {
        if let SessionEvent::Transcript(lines) = &e {
            if let Some(l) = lines.first() {
                task_line = l.id;
            }
        }
        sink(e);
    }
    // 本轮派发的任务必须原样保留，更早的内容才进摘要。
    maybe_compact(
        s,
        identity,
        if task_line > 0 { Some(task_line) } else { None },
        sink,
    );
    rounds_events(s, identity, live, sink);
}

/// 目的：讨论席的一个**成员回合**：与单 agent / 节点**同一条轮循环**（见 TurnRun），
///   差别只有三样：身份块、这一回合的工具面（角色表发放）、本回合提示；外加认协作表态（动词）。
///   产出落进**本会话**（这是它在这场工作里的经历）：回合标记（系统行）+ 逐轮的权威行；
///   返回这一回合的结论（表态 / 正文 / 思维链）交主会话收下（核实行留在它自己的会话里）。
#[allow(clippy::too_many_arguments)]
pub fn discussion_turn(
    s: &mut AgentSession,
    identity: &str,
    face: (Vec<String>, bool),
    turn: Vec<Msg>,
    turn_id: u64,
    round: usize,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) -> Result<MemberTurn, String> {
    maybe_compact(s, identity, None, sink);
    // 回合标记是**系统消息**（不是谁说的）：进上下文与转录都按 system，回档按同一口径还原。
    for e in s.note_system(&format!("[回合 t{}｜第 {} 轮]", turn_id, round)) {
        sink(e);
    }
    let spec = TurnRun {
        identity,
        face: Some(face),
        turn,
        verbs: true,
        turn_id: Some(turn_id),
    };
    let rounds = run_rounds(s, &spec, live, sink);
    if live.cancelled() {
        return Err("已停止".to_string());
    }
    // 末轮就是这一回合的结论（表态轮，或"没表态"的散文轮）；调用失败如实报错。
    let last = rounds.last().ok_or_else(|| "已停止".to_string())?;
    if let Some(err) = &last.error {
        return Err(err.clone());
    }
    Ok(MemberTurn::verdict(
        last.verb,
        last.text.clone(),
        last.degraded,
        last.truncated(),
    ))
}

/// 目的：继续：末条已是用户发言，直接用现有对话问模型（不新增用户消息）。
pub fn continue_reply(
    s: &mut AgentSession,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
    maybe_compact(s, identity, None, sink);
    rounds_events(s, identity, live, sink);
}

/// 单 agent / 节点的一回合：**同一条轮循环**（见 TurnRun），工具面用会话自己的（执行席）。
/// 讨论席的成员回合（`discussion_turn`）只是换了 TurnRun 的几个参数——没有第二条循环。
fn rounds_events(
    s: &mut AgentSession,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
    let spec = TurnRun {
        identity,
        face: None,
        turn: Vec::new(),
        verbs: false,
        turn_id: None,
    };
    run_rounds(s, &spec, live, sink);
}

/// 把一次问询的逐轮产出落成转录行：一轮的正文/思维链出文本行，工具另占一条工具行。
/// marks 逐行精确（回档按行截断）；工具轮的文本行与工具行同属一轮，
/// 所以历史统一在工具行推进（这一轮只贡献 assistant(raw) + [工具结果]），实时与重建两边一致。
/// 逐轮外送：**一轮跑完就出这一轮的行**（攒到回合收尾才一次性出，工具轮会把上一轮的
/// 流式文本从界面上抹掉）。行在回调里**只构造一次**；`run` 返回后只补记账——`marks` 是回档
/// 依据，必须保持"文本行的 mark 在 text_msgs 之前、工具行的 mark 在两个 msgs 之后"这个原时序。
fn run_rounds(
    s: &mut AgentSession,
    spec: &TurnRun<'_>,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    let label = s.id.clone();
    // 取消位在整段回合里只读地观察：每一轮构造行/收尾时都重新读，不能用开始时的快照。
    let cancel = std::sync::Arc::clone(&live.cancel);
    let next_line = std::cell::Cell::new(s.next_line);
    // 这次问询第一行会落在哪个 id：usage 事件跟着它，回档到它之前时用量一起失效。
    let usage_line = next_line.get();
    let per_round: std::cell::RefCell<Vec<Vec<LineView>>> = std::cell::RefCell::new(Vec::new());
    let error: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
    let mut on_round = |round: &Round, s: &mut dyn FnMut(SessionEvent)| {
        if let Some(err) = round.error.clone() {
            *error.borrow_mut() = Some(err);
            return;
        }
        let views = build_round_lines(
            &label,
            round,
            cancel.load(std::sync::atomic::Ordering::Relaxed),
            &next_line,
            spec.turn_id,
        );
        if !views.is_empty() {
            s(SessionEvent::Transcript(views.clone()));
        }
        per_round.borrow_mut().push(views);
    };
    let rounds = run(s, spec, live, &mut on_round, sink);
    s.next_line = next_line.get();

    // 只补记账（不构造行、不外送）：顺序与出账时序一致。
    for (round, views) in rounds.iter().zip(per_round.into_inner()) {
        if error.borrow().is_some() {
            break;
        }
        s.cur_reply = round.reply;
        // 读台账：一条读事实跟着它那条工具结果消息的正文走（发送视图据此判过期）。
        for v in &views {
            if let Some(c) = v.tool.as_ref() {
                if let Some(fact) = c.read.clone() {
                    s.note_read_at(crate::capabilities::session::api::tool_result_body(c), fact);
                }
            }
        }
        let has_line = !round.text.trim().is_empty() || !round.reasoning.trim().is_empty();
        let mut it = views.into_iter();
        match &round.tool {
            Some(run) => {
                if has_line {
                    if let Some(v) = it.next() {
                        s.line_reply.push(v.reply);
                        s.line_ids.push(v.id);
                        s.marks.push(s.dialogue.len());
                    }
                }
                for m in &round.text_msgs {
                    s.dialogue.push(m.clone());
                }
                for m in &run.msgs {
                    s.dialogue.push(m.clone());
                }
                if let Some(v) = it.next() {
                    s.line_reply.push(v.reply);
                    s.line_ids.push(v.id);
                    s.marks.push(s.dialogue.len());
                }
            }
            None => {
                // 与出账同一时序：先扩展历史，再记这一行的 mark。
                for m in &round.text_msgs {
                    s.dialogue.push(m.clone());
                }
                if let Some(v) = it.next() {
                    s.line_reply.push(v.reply);
                    s.line_ids.push(v.id);
                    s.marks.push(s.dialogue.len());
                }
            }
        }
    }
    // 至少有一次成功调用：更新用量（供应商精确 / 字符估算），状态栏据此刷新。
    // 全失败时不更新——别把上一次/上一次成功的数字冒充成这一次的。
    if rounds.iter().any(|r| r.error.is_none()) {
        let usage = s
            .chat
            .last_usage()
            .map(|t| {
                crate::capabilities::session::api::ContextUsage::from_tokens(t, s.context_window)
            })
            .unwrap_or_else(|| s.estimate_usage());
        s.set_last_usage(Some(usage));
        sink(SessionEvent::Usage {
            usage,
            line: usage_line,
        });
    }
    // 停止优先于同一次调用的错误：用户按了停止就是「我停的」，不再报成故障。
    let stopped = cancel.load(std::sync::atomic::Ordering::Relaxed);
    if stopped {
        for ev in s.note_notice(&crate::capabilities::session::api::stopped_note()) {
            sink(ev);
        }
    } else if let Some(err) = error.borrow().as_ref() {
        for ev in s.note_notice(&crate::capabilities::session::api::interrupted_note(err)) {
            sink(ev);
        }
    }
    rounds
}

/// 目的：发给模型的对话视图——过期读取换成占位符，其余原样。
///
/// 约束：只改**派生视图**，不动 `dialogue` 与转录。开关关、没有工作区、判定失败三者都原样返回——
///   判定失败宁可供旧内容，也不悄悄丢内容。只判主副本（`worktree` 为空的读）；私有沙箱里的读判不了就不动。
fn send_view(s: &AgentSession) -> Vec<crate::capabilities::llm::api::Msg> {
    let mut msgs = s.dialogue.clone();
    if !s.context_freshness() || s.reads().is_empty() {
        return msgs;
    }
    let Some(tools) = s.tools.as_ref() else {
        return msgs;
    };
    let Some(ws) = tools.workspace.as_ref() else {
        return msgs;
    };
    let reads: Vec<&crate::capabilities::session::api::ReadAt> = s
        .reads()
        .iter()
        .filter(|r| r.fact.worktree.is_empty())
        .collect();
    let queries: Vec<crate::capabilities::workspace::api::ReadQuery> = reads
        .iter()
        .map(|r| crate::capabilities::workspace::api::ReadQuery {
            path: r.fact.path.clone(),
            blob: r.fact.blob.clone(),
            from: r.fact.from,
            to: r.fact.to,
        })
        .collect();
    if queries.is_empty() {
        return msgs;
    }
    let Ok(stale) = ws.work_stale_reads(&tools.sandbox.work_name, &queries) else {
        return msgs;
    };
    let judged: Vec<(&crate::capabilities::session::api::ReadAt, bool)> =
        reads.into_iter().zip(stale).collect();
    msgs = crate::capabilities::session::api::freshness_view(&msgs, &judged);
    msgs
}

/// 以现有对话跑一次工具循环；流式时逐片外送短暂 Delta（信封正文不外流，避免糊屏）。
/// identity = 本回合的身份块（由驱动按当前提示词册现渲染；不进对话）。
fn run(
    s: &mut AgentSession,
    spec: &TurnRun<'_>,
    live: &mut Live,
    on_round: &mut crate::capabilities::collab::service::round::RoundSink<'_>,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    let label = s.id.clone();
    let llm = live.llm.clone();
    let cancel = std::sync::Arc::clone(&live.cancel);
    // 本回合的工具面（角色表发放）：**按回合换**——同一个会话会用两种身份干活（说话 / 干活）。
    // 装进这一回合的环境（声明与执行都读它），跑完还原。
    let saved = match spec.face.as_ref().zip(s.tools.as_mut()) {
        Some(((ids, with_modules), t)) => Some((
            std::mem::replace(&mut t.allowed, ids.clone()),
            std::mem::replace(&mut t.with_modules, *with_modules),
        )),
        None => None,
    };
    // 工具级确认：这一回合的裁决队（与核心各关卡**共用同一条队**，没有就跟平时一样直接执行）。
    let decisions = live.decisions.clone();
    let agent = s.params().agent.clone();
    let mut confirm = |req: &crate::capabilities::collab::service::tool_loop::ToolConfirm,
                       sink: &mut dyn FnMut(SessionEvent)|
     -> String {
        use crate::capabilities::session::api::{
            qualify, AnswerSlot, Pending, SlotWake, OPT_TOOL_ALLOW, OPT_TOOL_ALLOW_TURN,
            OPT_TOOL_DENY,
        };
        let Some(door) = decisions.as_ref() else {
            return OPT_TOOL_DENY.to_string();
        };
        let name = qualify(req.module.as_deref(), &req.tool);
        // 待确认落一条系统行（可回放），再进队并推卡片——**先登记再推**：
        // 否则用户手快会答在一张还没进队的卡上，答案丢掉、生成干等。
        sink(SessionEvent::Notice(format!(
            "[待确认] 工具 {} 需要用户放行。",
            name
        )));
        let slot = AnswerSlot::new();
        let (_, evs) = door.push(
            None,
            Pending::ToolApproval {
                agent: agent.clone(),
                module: req.module.clone(),
                tool: req.tool.clone(),
                args: req.args.clone(),
            },
            "",
            Some(std::sync::Arc::clone(&slot)),
        );
        for e in evs {
            sink(e);
        }
        // 等用户按卡上的选项作答（**不设超时**）：回答由核心写进这一格；整队作废（停止）= 拒绝。
        let answer = match slot.wait() {
            SlotWake::Answer(id) => id,
            _ => OPT_TOOL_DENY.to_string(),
        };
        sink(SessionEvent::Notice(format!(
            "[确认] 工具 {}：{}",
            name,
            match answer.as_str() {
                OPT_TOOL_ALLOW => "用户放行",
                OPT_TOOL_ALLOW_TURN => "用户放行，本轮不再询问",
                _ => "用户拒绝（或生成已停止）",
            }
        )));
        answer
    };
    // 只有接上裁决队时才建确认通道；否则本回合不接确认（照常执行）。
    let mut confirms =
        decisions.as_ref().map(
            |_| crate::capabilities::collab::service::tool_loop::ConfirmGate {
                allow_turn: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                confirm: &mut confirm,
            },
        );
    // 两个回调（流式分片 / 工具完成）都要外送短暂事件：把 emit 借出来共享（顺序因此天然正确）。
    let emit = std::cell::RefCell::new(&mut *live.emit);
    let mut acc = String::new();
    // 发给模型的对话视图：开着过期清理时，过期读取已换成占位符（见 send_view）。
    let view = send_view(s);
    let rounds = {
        let AgentSession { chat, tools, .. } = s;
        crate::capabilities::collab::service::round::converse_with(
            chat.as_mut(),
            tools.as_mut(),
            spec.identity,
            view,
            llm,
            &label,
            std::sync::Arc::clone(&cancel),
            &mut |chunk| {
                let mut kind = "text";
                let mut piece = String::new();
                match &chunk {
                    Chunk::Start => {
                        acc.clear();
                        kind = "start";
                    }
                    Chunk::Text(t) => {
                        let (send, next) = stream_piece(&acc, t);
                        piece = send;
                        acc = next;
                    }
                    Chunk::Reasoning(r) => {
                        kind = "reasoning";
                        piece = r.clone();
                    }
                }
                // 空片段不刷事件流：信封被抑制、重复快照合并掉的片都不该让前端每秒收几十条空 delta。
                if delta_visible(kind, &piece) {
                    (emit.borrow_mut())(SessionEvent::Delta {
                        speaker: label.clone(),
                        kind: kind.to_string(),
                        text: piece,
                    });
                }
                !cancel.load(std::sync::atomic::Ordering::Relaxed)
            },
            &mut |view: &ToolCallView| {
                (emit.borrow_mut())(SessionEvent::ToolCall(view.clone()));
            },
            on_round,
            sink,
            confirms.take(),
            // 工具层提问端口（围栏这类"装不上就先问"）：与工具级确认走**同一条队**。
            live.ask.as_deref(),
            &spec.turn,
            spec.verbs,
        )
    };
    if let (Some(t), Some((allowed, with_modules))) = (s.tools.as_mut(), saved) {
        t.allowed = allowed;
        t.with_modules = with_modules;
    }
    rounds
}
