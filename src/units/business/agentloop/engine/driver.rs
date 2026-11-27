//! 目的：成员回合驱动——在一条 AgentSession 上跑一个回合（单 agent 的话 / 子会话的任务 / 讨论席的发言 / 压缩）。
//! 管：say / dispatch_task / discussion_turn / continue_reply / compact_session 五个方法，以及它们用的私有装配（压缩、发送视图、轮循环接线）。
//! 不管：会话类型与状态（归会话单元的 domain）；轮循环本体（在 round.rs）；工具循环（在 tools.rs）；裁决队与提问端口的形状（在 io.rs 与各自端口后）。
//! 联动：turn 能力的生成流程取 AgentSession 后调这五个方法；契约见 docs/session/session-model.md。

use super::io::{CompactedTurn, MemberTurn, TurnIo};
use super::round::{Round, RoundSink, assemble, build_round_lines, converse_with};
use crate::units::business::agentloop::api::{AgentSession, TurnRun, delta_visible, stream_piece};
use crate::units::business::llm::api::{Chunk, Msg};
use crate::units::business::transcript::api::{LineView, SessionEvent, ToolCallView};
use std::sync::atomic::AtomicBool;

/// 目的：只把被总结的前缀发给模型、只声明 compact 工具，跑一次模型并返回摘要。
/// 约束：两条通道都认：native 从结构化槽位取，信封通道从正文里的信封取（与讨论回合同口径）。
pub fn compact_turn(
    s: &mut AgentSession,
    prompt: &str,
    plan: &crate::units::business::agentloop::api::CompactionPlan,
    identity: &str,
) -> Result<String, String> {
    let decl = s
        .tools
        .as_ref()
        .and_then(|t| t.face.builtin_tools.get("compact"))
        .map(|t| t.decl("compact"));
    let mut spec = s.compact_chat()?;
    let thinking = if spec.is_some() { None } else { s.thinking() };
    let cut = plan.cut.min(s.dialogue.len());
    // 整条消息**只有这一处装配**：身份 + 本回合工具（只有 compact）+ 被总结前缀 + 压缩提示。
    let msgs = assemble(
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
        crate::units::business::llm::api::CompleteOpts::plain(false).with_thinking(thinking);
    if let Some(d) = decl.as_ref() {
        opts.tools = Some(std::slice::from_ref(d));
    }
    let mut keep = |_c: crate::units::business::llm::api::Chunk| true;
    let chat: &mut crate::units::business::llm::api::BoxedChat = match spec.as_mut() {
        Some(c) => &mut c.chat,
        None => &mut s.chat,
    };
    let done = chat.complete(&msgs, opts, &mut keep);
    // 压缩调用也是真实花费：并进会话的累计（下一轮据此不再重复触发压缩的那份估算不在此列）。
    if let Some(u) = done.usage {
        s.note_usage(u);
    }
    if let Some(err) = done.error {
        return Err(err);
    }
    let (name, args) = match done.calls.first() {
        Some(c) => (c.name.clone(), c.args_json.clone()),
        None => {
            let r = crate::units::business::llm::api::parse(&done.raw);
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
    match s.compact_session(identity, protect_from) {
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

impl AgentSession {
    /// 目的：发言：先把 @ 引用改写成寻址 → 压入用户消息 → 逐轮（文本行 / 工具行）落转录。
    ///   改写在这一处完成，所以转录行与进上下文的消息是同一份文本（转录即内容）。
    pub fn say(
        &mut self,
        text: &str,
        identity: &str,
        io: &TurnIo,
        cancel: &AtomicBool,
        emit: &mut dyn FnMut(SessionEvent),
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        let text =
            crate::units::business::prompt::api::rewrite(text, Some(&self.id), &self.roots());
        self.dialogue.push(Msg::user(text.clone()));
        // 用户行不属于任何模型回复：给它**自己的行号**当回复号（与重建时的规则一致），
        // 否则它会继承上一轮的回复号，回档时与上一轮误并成一组。
        self.cur_reply = self.next_line;
        let user_line = self.line(text.to_string(), "user", "用户", "", None, None);
        let user_line_id = user_line.id;
        sink(SessionEvent::Transcript(vec![user_line]));
        // 到点先压一次：本轮刚收到的用户消息必须原样保留，更早的内容才进摘要。
        maybe_compact(self, identity, Some(user_line_id), sink);
        rounds_events(self, identity, io, cancel, emit, sink);
    }

    /// 目的：**派发并跑这一回合**：注入任务（`note_task`）后正常问模型。
    ///   节点派发只有这一条语义——CLI 与 Web 各自只是"点火"，不各写一套（见 session-model.md 四之二）。
    pub fn dispatch_task(
        &mut self,
        text: &str,
        identity: &str,
        io: &TurnIo,
        cancel: &AtomicBool,
        emit: &mut dyn FnMut(SessionEvent),
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        let mut task_line = 0u64;
        for e in self.note_task(text) {
            if let SessionEvent::Transcript(lines) = &e
                && let Some(l) = lines.first()
            {
                task_line = l.id;
            }
            sink(e);
        }
        // 本轮派发的任务必须原样保留，更早的内容才进摘要。
        maybe_compact(
            self,
            identity,
            if task_line > 0 { Some(task_line) } else { None },
            sink,
        );
        rounds_events(self, identity, io, cancel, emit, sink);
    }

    /// 目的：讨论席的一个**成员回合**：与单 agent / 节点**同一条轮循环**（见 TurnRun），
    ///   差别只有三样：身份块、这一回合的段与角色（工具面由它现拼）、本回合提示；外加认协作表态（动词）。
    ///   产出落进**本会话**（这是它在这场工作里的经历）：回合标记（系统行）+ 逐轮的权威行；
    ///   返回这一回合的结论（表态 / 正文 / 思维链）交主会话收下（核实行留在它自己的会话里）。
    #[allow(clippy::too_many_arguments)]
    pub fn discussion_turn(
        &mut self,
        identity: &str,
        loop_id: &str,
        role: &str,
        turn: Vec<Msg>,
        turn_id: u64,
        round: usize,
        io: &TurnIo,
        cancel: &AtomicBool,
        emit: &mut dyn FnMut(SessionEvent),
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<MemberTurn, String> {
        maybe_compact(self, identity, None, sink);
        // 回合标记是**系统消息**（不是谁说的）：进上下文与转录都按 system，回档按同一口径还原。
        for e in self.note_system(&format!("[回合 t{}｜第 {} 轮]", turn_id, round)) {
            sink(e);
        }
        let spec = TurnRun {
            identity,
            loop_id,
            role,
            turn,
            verbs: true,
            turn_id: Some(turn_id),
        };
        let rounds = run_rounds(self, &spec, io, cancel, emit, sink);
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
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
        &mut self,
        identity: &str,
        io: &TurnIo,
        cancel: &AtomicBool,
        emit: &mut dyn FnMut(SessionEvent),
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        maybe_compact(self, identity, None, sink);
        rounds_events(self, identity, io, cancel, emit, sink);
    }

    /// 目的：压一次：算切分点 → 只把被总结前缀交给模型 → 应用并落压缩记录行。
    /// 返回：没有可压缩内容时 `None`（调用方决定提示还是静默跳过）。
    /// 错误：模型调用失败 / 摘要为空时如实返回原因。
    pub fn compact_session(
        &mut self,
        identity: &str,
        protect_from: Option<u64>,
    ) -> Result<Option<CompactedTurn>, String> {
        let Some(plan) = self.plan_compaction(protect_from) else {
            return Ok(None);
        };
        let prompt = self.compact_prompt.clone();
        let summary = compact_turn(self, &prompt, &plan, identity)?;
        let Some(line) = self.apply_compaction(&plan, &summary) else {
            return Ok(None);
        };
        let usage = self.estimate_usage();
        self.set_last_usage(Some(usage));
        Ok(Some(CompactedTurn { line, usage }))
    }
}

/// 单 agent / 节点的一回合：**同一条轮循环**（见 TurnRun），身份就是会话当前坐的那一段与角色。
/// 讨论回合（`discussion_turn`）只是换了 TurnRun 的几个参数——没有第二条循环。
fn rounds_events(
    s: &mut AgentSession,
    identity: &str,
    io: &TurnIo,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(SessionEvent),
    sink: &mut dyn FnMut(SessionEvent),
) {
    // 会话当前坐的段与角色原样传回去：换与还原因此对任何回合都是同一个动作。
    let (loop_id, role) = (s.params().loop_id.clone(), s.role().to_string());
    let spec = TurnRun {
        identity,
        loop_id: &loop_id,
        role: &role,
        turn: Vec::new(),
        verbs: false,
        turn_id: None,
    };
    run_rounds(s, &spec, io, cancel, emit, sink);
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
    io: &TurnIo,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(SessionEvent),
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    let label = s.id.clone();
    // 取消位在整段回合里只读地观察：每一轮构造行/收尾时都重新读，不能用开始时的快照。
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
        // 中断标记由轮自己带（round.cut）：取消位是**这次生成**的状态，不是"构造这一行时"的状态——
        // 用它判标记会把恰好收完的那一轮也打成半句。
        let views = build_round_lines(&label, round, &next_line, spec.turn_id);
        if !views.is_empty() {
            s(SessionEvent::Transcript(views.clone()));
        }
        per_round.borrow_mut().push(views);
    };
    let rounds = run(s, spec, io, cancel, emit, &mut on_round, sink);
    s.next_line = next_line.get();

    // 这一回合每一轮（工具轮、被截断的轮、失败轮都算只记它实际报回的）的精确用量并进累计：
    // 回合可能调用多次模型，`last_usage` 只留最后一次，账面必须逐轮加。
    for r in &rounds {
        if let Some(u) = r.usage {
            s.note_usage(u);
        }
    }

    // 只补记账（不构造行、不外送）：顺序与出账时序一致。
    for (round, views) in rounds.iter().zip(per_round.into_inner()) {
        if error.borrow().is_some() {
            break;
        }
        s.cur_reply = round.reply;
        // 读台账：完整读的观察事实跟着它那条工具结果消息的正文走（发送视图据此判过期）。
        for c in views.iter().filter_map(|v| v.tool.as_ref()) {
            let body = crate::units::business::agentloop::api::tool_result_body(c);
            for fact in &c.observations {
                for read in fact.read_facts() {
                    s.note_read_at(body.clone(), fact.reply, read);
                }
            }
        }
        let has_line = !round.text.trim().is_empty() || !round.reasoning.trim().is_empty();
        let mut it = views.into_iter();
        match &round.tool {
            Some(run) => {
                if has_line && let Some(v) = it.next() {
                    s.line_reply.push(v.reply);
                    s.line_ids.push(v.id);
                    s.marks.push(s.dialogue.len());
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
                crate::units::business::transcript::api::ContextUsage::from_tokens(
                    t,
                    s.context_window,
                )
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
        for ev in s.note_notice(&crate::units::business::transcript::api::stopped_note()) {
            sink(ev);
        }
    } else if let Some(err) = error.borrow().as_ref() {
        for ev in s.note_notice(&crate::units::business::transcript::api::interrupted_note(
            err,
        )) {
            sink(ev);
        }
    }
    rounds
}

/// 目的：发给模型的对话视图——过期读取换成占位符，其余原样；同时把这次替换的统计写回会话。
///
/// 约束：只改**派生视图**，不动 `dialogue` 与转录。开关关、没有工作区、判定失败三者都原样返回——
///   判定失败宁可供旧内容，也不悄悄丢内容。主副本的读对照主线树，私有沙箱的读对照沙箱当前内容。
///   保底：前 freshness_keep_percent 的内容原样保留（前缀缓存靠它保住），只替换其后的过期读。
fn send_view(s: &mut AgentSession) -> Vec<crate::units::business::llm::api::Msg> {
    let computed = freshness_now(s);
    s.set_freshness_stats(computed.as_ref().map(|(_, stats)| *stats));
    match computed {
        Some((view, _)) => view,
        None => s.dialogue.clone(),
    }
}

/// 目的：算这一次的发送视图与替换统计；None = 不做替换（开关关 / 没有工作区 / 判定失败）。
fn freshness_now(
    s: &AgentSession,
) -> Option<(
    Vec<crate::units::business::llm::api::Msg>,
    crate::units::business::agentloop::api::FreshnessStats,
)> {
    if !s.context_freshness() || s.reads().is_empty() {
        return None;
    }
    let tools = s.tools.as_ref()?;
    let driver = tools.driver.as_ref()?;
    let reads: Vec<&crate::units::business::agentloop::api::ReadAt> = s.reads().iter().collect();
    let queries: Vec<crate::units::business::workbench::api::ReadQuery> = reads
        .iter()
        .map(|r| crate::units::business::workbench::api::ReadQuery {
            path: r.fact.path.clone(),
            blob: r.fact.blob.clone(),
            from: r.fact.from,
            to: r.fact.to,
            // worktree 记的是读发生在哪个沙箱（空 = 主副本）。
            sandbox: !r.fact.worktree.is_empty(),
        })
        .collect();
    if queries.is_empty() {
        return None;
    }
    // 占位符策略：只有内容真变了（Changed）才换；版本变了但那段内容还在（Shifted）照旧供着。
    let stale: Vec<bool> = driver.stale_reads(
        &tools.face.bench.work_name,
        &tools.face.bench.agent,
        &queries,
    )?;
    let judged: Vec<(&crate::units::business::agentloop::api::ReadAt, bool)> =
        reads.into_iter().zip(stale).collect();
    // 保底按**字符**算（与压缩估算同一口径：4 字符 ≈ 1 token）。
    let total_chars: usize = s.dialogue.iter().map(|m| m.content.chars().count()).sum();
    let keep_percent = s.freshness_keep_percent();
    let keep_chars = total_chars * keep_percent as usize / 100;
    let (view, mut stats) =
        crate::units::business::agentloop::api::freshness_view(&s.dialogue, &judged, keep_chars);
    stats.keep_percent = keep_percent;
    Some((view, stats))
}

/// 以现有对话跑一次工具循环；流式时逐片外送短暂 Delta（信封正文不外流，避免糊屏）。
/// identity = 本回合的身份块（由驱动按当前提示词册现渲染；不进对话）。
fn run(
    s: &mut AgentSession,
    spec: &TurnRun<'_>,
    io: &TurnIo,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(SessionEvent),
    on_round: &mut RoundSink<'_>,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    let label = s.id.clone();
    let llm = io.llm.clone();
    // 本回合的身份与工具面：**按段与角色一起换**。同一个会话会用两种身份干活（说话 / 干活），只换面不换身份
    // 会让提示词还停在上一身份；`enter_identity` 让会话的装配机现拼这一份（声明与执行都读它），跑完一并还原。
    let (saved_loop, saved_role) = (s.params().loop_id.clone(), s.role().to_string());
    s.enter_identity(spec.loop_id, spec.role);
    // 两个回调（流式分片 / 工具完成）都要外送短暂事件：把 emit 借出来共享（顺序因此天然正确）。
    let emit = std::cell::RefCell::new(emit);
    let mut acc = String::new();
    // 发给模型的对话视图：开着过期清理时，过期读取已换成占位符（见 send_view）。
    let view = send_view(s);
    let window = s.context_window();
    let rounds = {
        let AgentSession { chat, tools, .. } = s;
        converse_with(
            chat.as_mut(),
            tools.as_mut(),
            spec.identity,
            view,
            llm,
            window,
            &label,
            cancel,
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
            // 工具层提问端口（围栏这类"装不上就先问"）：用户回答走会话的裁决队。
            io.ask.as_deref(),
            &spec.turn,
            spec.verbs,
        )
    };
    s.enter_identity(&saved_loop, &saved_role);
    rounds
}
