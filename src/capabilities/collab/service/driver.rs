//! **回合驱动**：在一条 `AgentSession` 上跑一个回合（单 agent 的话 / 子会话的任务 / 讨论席的发言 / 压缩）。
//!
//! 为什么是**自由函数**而不是 `impl AgentSession`：会话类型归会话能力、引擎归协作能力——
//! 给别人的类型写 `impl` 是另一种互相引入（R1），所以驱动以「会话当参数」的形式写在这里。

use super::engine::*;
use crate::capabilities::llm::api::{Chunk, Msg};
use crate::capabilities::session::api::{
    stream_piece, AgentSession, LineView, Live, SessionEvent, ToolCallView, TurnRun,
};
/// 压缩回合：把提示词追加到历史之后、**只声明 compact 工具**，跑一次模型；拿到摘要就返回。
/// 两条通道都认：native 从结构化槽位取，信封通道从正文里的信封取（与讨论回合同口径）。
pub fn compact_turn(
    s: &mut AgentSession,
    prompt: &str,
    decl: Option<&crate::capabilities::llm::api::ToolDecl>,
    identity: &str,
) -> Result<String, String> {
    // 整条消息**只有这一处装配**：身份 + 本回合工具（只有 compact）+ 对话 + 压缩提示。
    let msgs = crate::capabilities::collab::service::engine::assemble(
        identity,
        s.tools.as_ref(),
        &["compact".to_string()],
        false,
        &s.dialogue,
        &[Msg::user(prompt.to_string())],
    );
    let mut opts = crate::capabilities::llm::api::CompleteOpts::plain(false);
    if let Some(d) = decl {
        opts.tools = Some(std::slice::from_ref(d));
    }
    let mut keep = |_c: crate::capabilities::llm::api::Chunk| true;
    let done = s.chat.complete(&msgs, opts, &mut keep);
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

/// 到点自动压一次：估算历史字符数（≈ tokens × 4），超预算就跑一个压缩回合。
/// 压不动就**如实通知并继续用完整上下文**（不静默降级、不假装压过）。
fn maybe_compact(s: &mut AgentSession, identity: &str, sink: &mut dyn FnMut(SessionEvent)) {
    if s.compact_at == 0 {
        return;
    }
    let chars: usize = s.dialogue.iter().map(|m| m.content.chars().count()).sum();
    if chars <= s.compact_at {
        return;
    }
    let decl = s
        .tools
        .as_ref()
        .and_then(|t| t.builtin_tools.get("compact"))
        .map(|s| s.decl("compact"));
    let prompt = s.tool_texts.compact_prompt.clone();
    let up_to = s.next_line;
    match compact_turn(s, &prompt, decl.as_ref(), identity) {
        Ok(summary) => {
            s.compact(up_to, &summary);
            sink(SessionEvent::Compacted { up_to, summary });
        }
        Err(err) => sink(SessionEvent::Notice(format!(
            "[警告] 自动压缩没成功：{}（继续用完整上下文）",
            err
        ))),
    }
}

/// 发言：先把 @ 引用改写成寻址 → 压入用户消息 → 逐轮（文本行 / 工具行）落转录。
/// 改写在这一处完成，所以转录行与进上下文的消息是同一份文本（转录即内容）。
pub fn say(
    s: &mut AgentSession,
    text: &str,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
    // 到点先压一次：**同一个工作线程内**跑，不阻塞核心。
    maybe_compact(s, identity, sink);
    let text = crate::capabilities::prompt::api::rewrite(text, Some(&s.id), &s.roots(), &s.refs);
    s.dialogue.push(Msg::user(text.clone()));
    // 用户行不属于任何模型回复：给它**自己的行号**当回复号（与重建时的规则一致），
    // 否则它会继承上一轮的回复号，回档时与上一轮误并成一组。
    s.cur_reply = s.next_line;
    let user_line = s.line(text.to_string(), "user", "用户", "", None, None);
    sink(SessionEvent::Transcript(vec![user_line]));
    rounds_events(s, identity, live, sink);
}

/// **派发并跑这一回合**：注入任务（`note_task`）后正常问模型。
/// 节点派发只有这一条语义——CLI 与 Web 各自只是"点火"，不各写一套（见 session-model.md 四之二）。
pub fn dispatch_task(
    s: &mut AgentSession,
    text: &str,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
    maybe_compact(s, identity, sink);
    for e in s.note_task(text) {
        sink(e);
    }
    rounds_events(s, identity, live, sink);
}

/// 讨论席的一个**成员回合**：与单 agent / 节点**同一条轮循环**（见 TurnRun），
/// 差别只有三样：身份块、这一回合的工具面（角色表发放）、本回合提示；外加认协作表态（动词）。
/// 产出落进**本会话**（这是它在这场工作里的经历）：回合标记（系统行）+ 逐轮的权威行；
/// 返回这一回合的结论（表态 / 正文 / 思维链）交主会话收下（核实行留在它自己的会话里）。
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

/// 继续：末条已是用户发言，直接用现有对话问模型（不新增用户消息）。
pub fn continue_reply(
    s: &mut AgentSession,
    identity: &str,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) {
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
/// 逐轮外送：**一轮跑完就出这一轮的行**（以前攒到回合收尾才一次性出，工具轮会把上一轮的
/// 流式文本从界面上抹掉）。行在回调里**只构造一次**；`run` 返回后只补记账——`marks` 是回档
/// 依据，必须保持"文本行的 mark 在 text_msgs 之前、工具行的 mark 在两个 msgs 之后"这个原时序。
fn run_rounds(
    s: &mut AgentSession,
    spec: &TurnRun<'_>,
    live: &mut Live,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    let label = s.id.clone();
    let texts = s.tool_texts.clone();
    let stopped = live.cancelled();
    let next_line = std::cell::Cell::new(s.next_line);
    let per_round: std::cell::RefCell<Vec<Vec<LineView>>> = std::cell::RefCell::new(Vec::new());
    let error: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
    let mut on_round = |round: &Round, s: &mut dyn FnMut(SessionEvent)| {
        if let Some(err) = round.error.clone() {
            *error.borrow_mut() = Some(err);
            return;
        }
        let views = build_round_lines(&label, &texts, round, stopped, &next_line, spec.turn_id);
        if !views.is_empty() {
            s(SessionEvent::Transcript(views.clone()));
        }
        per_round.borrow_mut().push(views);
    };
    let rounds = run(s, spec, live, &mut on_round, sink);
    s.next_line = next_line.get();

    // 只补记账（不再构造行、不再外送）：顺序与旧逻辑逐字对应。
    for (round, views) in rounds.iter().zip(per_round.into_inner()) {
        if error.borrow().is_some() {
            break;
        }
        s.cur_reply = round.reply;
        let has_line = !round.text.trim().is_empty() || !round.reasoning.trim().is_empty();
        let mut it = views.into_iter();
        match &round.tool {
            Some(run) => {
                if has_line {
                    if let Some(v) = it.next() {
                        s.line_reply.push(v.reply);
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
                    s.marks.push(s.dialogue.len());
                }
            }
            None => {
                // 与旧逻辑同一时序：先扩展历史，再记这一行的 mark。
                for m in &round.text_msgs {
                    s.dialogue.push(m.clone());
                }
                if let Some(v) = it.next() {
                    s.line_reply.push(v.reply);
                    s.marks.push(s.dialogue.len());
                }
            }
        }
    }
    if let Some(err) = error.borrow().as_ref() {
        sink(SessionEvent::Notice(
            crate::capabilities::session::api::interrupted_note(err),
        ));
    }
    if stopped {
        sink(SessionEvent::Notice(
            "[已停止] 生成已按你的要求中止（保留已产出的部分）".to_string(),
        ));
    }
    rounds
}

/// 以现有对话跑一次工具循环；流式时逐片外送短暂 Delta（信封正文不外流，避免糊屏）。
/// identity = 本回合的身份块（由驱动按当前提示词册现渲染；不进对话）。
fn run(
    s: &mut AgentSession,
    spec: &TurnRun<'_>,
    live: &mut Live,
    on_round: &mut crate::capabilities::collab::service::engine::RoundSink<'_>,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    let label = s.id.clone();
    let llm = live.llm;
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
    // 两个回调（流式分片 / 工具完成）都要外送短暂事件：把 emit 借出来共享（顺序因此天然正确）。
    let emit = std::cell::RefCell::new(&mut *live.emit);
    let mut acc = String::new();
    let rounds = {
        let AgentSession {
            dialogue,
            chat,
            tools,
            ..
        } = s;
        crate::capabilities::collab::service::engine::converse_with(
            chat.as_mut(),
            tools.as_mut(),
            spec.identity,
            dialogue.clone(),
            llm,
            &label,
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
                (emit.borrow_mut())(SessionEvent::Delta {
                    speaker: label.clone(),
                    kind: kind.to_string(),
                    text: piece,
                });
                !cancel.load(std::sync::atomic::Ordering::Relaxed)
            },
            &mut |view: &ToolCallView| {
                (emit.borrow_mut())(SessionEvent::ToolCall(view.clone()));
            },
            on_round,
            sink,
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
