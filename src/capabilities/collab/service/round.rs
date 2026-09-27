//! **轮循环与行构造**：一次成员回复的完整翻译（`converse_with`）、请求装配（`assemble`）、
//! 行构造（`build_round_lines`）与轮/回复词汇（`Round` / `verb_of` / `verb_tag` / `arg_text`）。
//!
//! 实时与重建都走这里：两边的消息与行必须逐条一致（唯一构造函数）。
//!
//! 状态机在 `discussion`。

use super::discussion::*;
use crate::capabilities::collab::service::tool_loop::*;
use crate::capabilities::llm::api::{self as envelope, Verb};
use crate::capabilities::llm::api::{Chat, Chunk, CompleteOpts, Msg};
use crate::capabilities::session::api::MemberTools;
use crate::capabilities::session::api::{reply_msgs, LineView, SessionEvent, ToolCallView};
use crate::capabilities::tools::api::ToolOutcome;

/// 一次工具调用的产出：调用视图 + 它压进历史的消息。
pub struct ToolRun {
    pub view: ToolCallView,
    /// 该工具行压进历史的消息（[工具结果] …）。
    pub msgs: Vec<Msg>,
}

/// 拼一次模型调用的消息：**身份 + 本回合工具 + 对话 + 本回合提示**。
///
/// 为什么只有这一处：身份与工具块都是**派生**的（登记处 + 提示词册 + 这一回合的身份），
/// 它们不占对话的位置——对话里只有真正发生过的事（谁说了什么、调了什么工具）。
/// 实时与重建都从这里拼，所以"回放与实时产出同样的消息"只约束对话本身。
pub fn assemble(
    identity: &str,
    tools: Option<&MemberTools>,
    ids: &[String],
    with_modules: bool,
    dialogue: &[Msg],
    turn: &[Msg],
) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::with_capacity(dialogue.len() + turn.len() + 2);
    out.push(Msg::system(identity));
    if let Some(ctx) = tools {
        let block = ctx.tools_block(ids, with_modules);
        if !block.is_empty() {
            out.push(Msg::system(block));
        }
    }
    out.extend(dialogue.iter().cloned());
    out.extend(turn.iter().cloned());
    out
}

/// 逐轮产出回调：拿到刚定稿的一轮 + 本次的出口。
/// 出口当参数传而不是让回调捕获它——否则回调借着 sink，调用方随后用不了同一个 sink。
pub type RoundSink<'a> = dyn FnMut(&Round, &mut dyn FnMut(SessionEvent)) + 'a;

/// 讨论行的**逐成员外送回调**：拿到刚定稿的行 + 本次的出口。
/// 出口当参数传而不是让回调捕获它——否则回调借着 sink，`step`/`open` 的调用方随后用不了它。
pub type LineSink<'a> = dyn FnMut(&[LineView], &mut dyn FnMut(SessionEvent)) + 'a;

/// 表态动词的**行标签**：讨论转录行写成 `[谁:say] 内容`（行格式与回档解析同一处定义）。
pub(crate) fn verb_tag(v: Verb) -> &'static str {
    match v {
        Verb::Say => "say",
        Verb::Ask => "ask",
        Verb::Leave => "leave",
        Verb::Agree => "agree",
        Verb::Tool => "tool",
    }
}

/// 原生通道的工具名 → 讨论动词：**只认协作动词**，其余一律不认识（不认识 = 越权，如实拒绝）。
pub fn verb_of(name: &str) -> Option<Verb> {
    match name {
        "say" => Some(Verb::Say),
        "agree" => Some(Verb::Agree),
        "leave" => Some(Verb::Leave),
        "ask" => Some(Verb::Ask),
        _ => None,
    }
}

/// 原生调用的参数里取正文（供应商给的是一段 JSON 文本；取不到就是空串——不猜）。
pub fn arg_text(args_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(args_json)
        .ok()
        .and_then(|v| {
            v.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 一轮模型调用的产出（一轮 = 一条文本转录行；有工具时紧跟一条工具行）。
/// 原始输出不进这里：工具轮由 ToolCallView.raw 承载、文本轮进上下文的就是解析后的文本。
pub struct Round {
    /// 这一轮属于哪次模型回复（一次回复可能产出多条工具行）。
    pub reply: u64,
    /// 解析后的可见文本（信封缺失时即原文）；工具轮为空串（它说的就是那封信封）。
    pub text: String,
    /// 该轮思维链（没给就是空串）。
    pub reasoning: String,
    /// 该轮压进历史的消息：工具轮 = [assistant(raw)]，末轮 = [assistant(text)]。
    pub text_msgs: Vec<Msg>,
    pub tool: Option<ToolRun>,
    /// 供应商给的结束原因（原样；没给 = 空串）：核心据此分辨"写完停"还是"被长度截断"。
    pub finish: String,
    /// 这次调用失败了（超时 / 网络）：非空 = **没有拿到模型回复**，这一轮不该落转录。
    /// 上层据此如实告知用户并中断本轮（用户可以点「继续」重试）。
    pub error: Option<String>,
    /// 这一轮是一次**表态**（讨论席的协作动词）：它是该回合的收尾轮，行上带动词标签。
    /// 执行席不表态（恒为 None）。
    pub verb: Option<Verb>,
    /// 这一轮的回复**没写成信封**（散文 / 信封写坏）：如实传给上层，不假装它是一次规范回复。
    pub degraded: bool,
}

impl Round {
    /// 这一轮的输出是不是被供应商按长度截断了。
    pub fn truncated(&self) -> bool {
        crate::capabilities::llm::api::truncated(&self.finish)
    }
}

/// stream/on 透传给通道（呈现层在 on 里外送 Delta）；on 返回 false = 用户要求中止。
/// on_tool 在每个工具跑完后立刻回调（工具行与文本行因此天然有序）。
/// 终止保证：超限后告知一次并强制收尾；其后再来 tool 信封按原文作答，不再执行。
// 逐轮外送要的四个出口（分片 / 工具 / 逐轮 / 提示词）都是回调，收口成参数对象只是把参数挪个地方、
// 并让"谁在什么时候拿到什么"更难读。这是有意的设计取舍（同 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::too_many_arguments)]
pub fn converse_with(
    chat: &mut dyn Chat,
    mut tools: Option<&mut MemberTools>,
    identity: &str,
    dialogue: Vec<Msg>,
    llm: crate::capabilities::llm::api::LlmOpts,
    speaker: &str,
    on: &mut dyn FnMut(Chunk) -> bool,
    on_tool: &mut dyn FnMut(&ToolCallView),
    on_round: &mut RoundSink<'_>,
    sink: &mut dyn FnMut(SessionEvent),
    // 本回合的提示（讨论席的开场/轮转词；执行席常为空——它的指令在派发行里）。
    turn: &[Msg],
    // 本回合认不认**协作表态**（讨论席认：见到动词就是这一回合的发言，收尾）。
    verbs: bool,
) -> Vec<Round> {
    // 身份 + 本回合工具 + 对话 + 本回合提示：**唯一的装配点**。
    // 工具面取自这一席位（执行席的表现 + 它自己模块的工具）；讨论席的动词面也在这里（见 crate::capabilities::session::api::TurnRun）。
    let (ids, with_modules) = match tools.as_ref() {
        Some(ctx) => (ctx.allowed.clone(), ctx.with_modules),
        None => (Vec::new(), false),
    };
    let mut msgs = assemble(
        identity,
        tools.as_deref(),
        &ids,
        with_modules,
        &dialogue,
        turn,
    );
    // 观察账本随会话保存（回档时清空），这里不动它——它的语义是"这一段转录里的读取证据"。
    let mut rounds: Vec<Round> = Vec::new();
    // 逐轮产出：**一轮跑完就把它交出去**（调用方据此立刻外送与落盘，不必等整个回合结束）。
    // 用宏而不是逐个改写 push 点：5 个分支都要"先回调、再入册"，写死五遍迟早漏一处。
    macro_rules! push_round {
        ($r:expr) => {{
            let r = $r;
            on_round(&r, sink);
            rounds.push(r);
        }};
    }
    // 没有工具环境时的回复号来源（见下面 reply_id）。
    let mut local_reply = 0u64;
    // 本代是否被用户中途停止（通道回调返回 false）：循环顶部据此退出（见下）。
    let mut aborted = false;
    loop {
        // 用户在生成中途点了「停止」（通道回调返回 false）：**这一轮到此为止**，不再发起下一次调用。
        // 没有这条出口，被停的生成会以"每次调用立刻返回"的速度空转（真机上烧过一次 CPU）；
        // 此前是靠工具调用上限兜底的——上限删掉后这条出口必须自己站住。
        if aborted {
            return rounds;
        }
        // 这一回复的稳定 id：一次模型回复一个号，本次问询里的多条工具行共用它。
        // 没有工具环境时给本代内的局部号即可（那条路径不落转录行、也不分组）。
        let reply_id = match tools.as_deref_mut() {
            Some(ctx) => ctx.next_reply(),
            None => {
                local_reply += 1;
                local_reply
            }
        };
        // 形态与工具声明面：由本成员的通道形态决定（envelope = 不声明，走手写信封；native = 声明本成员的工具）
        let (mode, decls) = match tools.as_deref_mut() {
            Some(ctx) if ctx.mode == crate::capabilities::llm::api::ToolMode::Native => {
                let mode = ctx.mode;
                (mode, tool_decls(ctx))
            }
            Some(ctx) => (ctx.mode, ToolDecls::default()),
            None => (
                crate::capabilities::llm::api::ToolMode::Envelope,
                ToolDecls::default(),
            ),
        };
        // 逐轮累积思维链（原文以通道返回值为准：非流式通道不回 Chunk）。
        let mut reasoning = String::new();
        let done = {
            let mut sink = |chunk: Chunk| {
                match &chunk {
                    Chunk::Start => reasoning.clear(),
                    Chunk::Text(_) => {}
                    Chunk::Reasoning(r) => reasoning.push_str(r),
                }
                let keep = on(chunk);
                if !keep {
                    aborted = true;
                }
                keep
            };
            let opts = CompleteOpts {
                stream: llm.stream,
                tools: if decls.list.is_empty() {
                    None
                } else {
                    Some(&decls.list)
                },
                timeout_secs: llm.timeout_secs,
            };
            chat.complete(&msgs, opts, &mut sink)
        };
        // 调用失败（超时 / 网络）：**不是模型的回复**——这一轮不解析信封、不执行工具、不落转录，
        // 只把原因带回，让上层如实告知用户并中断本轮（用户可以点「继续」重试）。
        // 为什么必须短路：错误文本若被当成发言吸收，核心按转录派生的"下一步该谁说话"就歪了。
        if let Some(err) = done.error.clone() {
            push_round!(Round {
                reply: reply_id,
                text: String::new(),
                reasoning: String::new(),
                text_msgs: Vec::new(),
                tool: None,
                finish: String::new(),
                error: Some(err),
                verb: None,
                degraded: false,
            });
            return rounds;
        }
        // 非流式供应商不回 Chunk，使用响应里的思维链。
        if reasoning.is_empty() {
            reasoning = done.reasoning.clone();
        }
        // 结束原因如实带回：被长度截断要落日志——事后才判定得出"是截断还是模型自己写错"。
        let finish = done.finish.clone();
        let truncated = done.truncated();
        let calls = done.calls.clone();
        let raw = done.raw;
        if truncated {
            if let Some(t) = tools.as_deref_mut() {
                t.log.warn(
                    "engine::converse",
                    &format!(
                        "模型输出被长度截断（finish_reason={}）：第 {} 轮，正文 {} 字",
                        finish,
                        rounds.len() + 1,
                        raw.chars().count()
                    ),
                );
            }
        }
        let mut reply = envelope::parse(&raw);
        // 手写信封不合法时**先**问修复端口：只做无歧义的修补（默认实现只转义字符串里的裸控制字符）。
        // 修好并重新解析成合法工具信封 = 本轮照常执行工具；修不了就走原来的"失败工具行"路径。
        // 封顶后不修（与"封顶后不再执行工具"同一口径）。
        // 用户中止的生成不修：半截信封是"被停下来"的产物，不是模型的意图——绝不据此执行工具。
        // 自由格式工具（patch）也不修：它的正文在信封之外，转义控制字符会把补丁里的换行弄坏。
        let freeform_tool = reply
            .tools
            .iter()
            .any(|t| crate::capabilities::tools::api::is_freeform(&t.name));
        let mut repaired: Option<String> = None;
        if !aborted && !freeform_tool {
            if let Some(kind) = reply.tools.first().and_then(|t| t.malformed.clone()) {
                if let Some(ctx) = tools.as_deref_mut() {
                    let out = ctx.llm.repair(&raw, &kind);
                    if let Some(text) = out.repaired.as_deref() {
                        let again = envelope::parse(text);
                        if !again.tools.is_empty()
                            && again.tools.iter().all(|t| t.malformed.is_none())
                        {
                            reply = again;
                            repaired = Some(out.what.join("；"));
                        }
                    }
                }
            }
        }
        // ── 讨论席的**表态**：这一轮就是它的发言——不执行任何调用，本回合到此收尾 ──
        // 与单 agent 的末轮同一形态（正文一行 + 历史一条 assistant），不同的只是行上带动词标签。
        // 两条通道各认各的：native 从结构化槽位认动词，信封从信封自己的动词字段认（散文不算表态）。
        if verbs {
            let from_envelope = |r: &envelope::Reply| -> Option<(Verb, String, bool)> {
                if r.degraded || r.verb == Verb::Tool {
                    None
                } else {
                    Some((r.verb, r.text.clone(), false))
                }
            };
            let said: Option<(Verb, String, bool)> =
                if mode == crate::capabilities::llm::api::ToolMode::Native {
                    calls
                        .iter()
                        .find_map(|c| verb_of(&c.name).map(|v| (v, arg_text(&c.args_json), false)))
                        .or_else(|| from_envelope(&reply))
                } else {
                    from_envelope(&reply)
                };
            if let Some((verb, text, degraded)) = said {
                let has_line = !text.trim().is_empty() || !reasoning.trim().is_empty();
                let text_msgs = if has_line {
                    vec![Msg::assistant(text.trim().to_string())]
                } else {
                    Vec::new()
                };
                push_round!(Round {
                    reply: reply_id,
                    text,
                    reasoning,
                    text_msgs,
                    tool: None,
                    finish,
                    error: None,
                    verb: Some(verb),
                    degraded,
                });
                return rounds;
            }
        }
        // ── 原生通道：工具调用来自供应商的结构化槽位（不解析信封）──
        if mode == crate::capabilities::llm::api::ToolMode::Native {
            if let Some(ctx) = tools.as_deref_mut() {
                // ① 有原生调用：逐个执行，各成一条工具行；助手消息如实记下"它调了什么"（回放与下一轮都看得到）
                if !calls.is_empty() {
                    // 先定好每个调用落在哪个工具、参数是什么（patch 的正文在 body 参数里，
                    // 原生协议要求参数是 JSON 对象；转义交给供应商的解码器）。
                    let plan: Vec<(Option<String>, String, String)> = calls
                        .iter()
                        .map(|c| {
                            let (module, tool) = decls
                                .wire
                                .get(&c.name)
                                .cloned()
                                .unwrap_or((None, c.name.clone()));
                            let args = if crate::capabilities::tools::api::is_freeform(&tool) {
                                serde_json::from_str::<serde_json::Value>(&c.args_json)
                                    .ok()
                                    .and_then(|v| {
                                        v.get("body")
                                            .and_then(|b| b.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_default()
                            } else {
                                c.args_json.clone()
                            };
                            (module, tool, args)
                        })
                        .collect();
                    // 调度：**连续**声明可并发的调用合成一批并发跑，其余各自独占（写入类因此是批次之间的屏障）。
                    // 结果按原始下标返回，随后一律按原序回填——并发只影响执行，不影响上下文里的顺序。
                    note_unauthorized(ctx, &plan, sink);
                    let done = run_batch(ctx, &plan);
                    // 先按原序把工具行建好（执行已经做完），再让**唯一那处**构造函数产出这一回复的消息：
                    // 实时与重建走同一个函数，"重建上下文与实时一致"因此是结构保证的。
                    let texts = &ctx.sandbox.texts;
                    let reply_text = reply.text.clone();
                    let views: Vec<ToolCallView> = calls
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let (label, outcome) = done[i].clone();
                            ToolCallView {
                                speaker: speaker.to_string(),
                                module: label,
                                name: plan[i].1.clone(),
                                ok: outcome.ok,
                                args: plan[i].2.clone(),
                                output: outcome.output,
                                // 助手消息正文 = 这一回复的原文（正文与调用进的是同一条消息）
                                raw: reply_text.clone(),
                                call_id: c.id.clone(),
                                reply: reply_id,
                            }
                        })
                        .collect();
                    let msgs_of = reply_msgs(ctx.mode, &reply_text, &views, texts);
                    for m in &msgs_of {
                        msgs.push(m.clone());
                    }
                    for (i, view) in views.into_iter().enumerate() {
                        on_tool(&view);
                        push_round!(Round {
                            reply: reply_id,
                            // 正文只挂在本回复的第一条工具行上（只显示一条，不重复）
                            text: if i == 0 {
                                reply_text.clone()
                            } else {
                                String::new()
                            },
                            reasoning: std::mem::take(&mut reasoning),
                            text_msgs: if i == 0 {
                                vec![msgs_of[0].clone()]
                            } else {
                                Vec::new()
                            },
                            tool: Some(ToolRun {
                                view,
                                msgs: vec![msgs_of[i + 1].clone()],
                            }),
                            finish: finish.clone(),
                            error: None,
                            verb: None,
                            degraded: false,
                        });
                    }
                    continue;
                }
                // ② 没有原生调用却写了信封：**不执行**（两套形态互斥），但也不静默丢掉意图
                {
                    if let Some(inv) = reply.tools.first().cloned() {
                        let texts = &ctx.sandbox.texts;
                        let view = ToolCallView {
                            speaker: speaker.to_string(),
                            module: inv.module.clone().unwrap_or_default(),
                            name: inv.name.clone(),
                            ok: false,
                            args: inv.args_json.clone(),
                            output: texts.native_no_envelope.clone(),
                            raw: raw.clone(),
                            call_id: String::new(),
                            reply: reply_id,
                        };
                        on_tool(&view);
                        // 这条没有合法原生 id（模型是手写的信封）：走文本形状，不能发 role=tool。
                        let msgs_of = reply_msgs(mode, &raw, std::slice::from_ref(&view), texts);
                        for m in &msgs_of {
                            msgs.push(m.clone());
                        }
                        push_round!(Round {
                            reply: reply_id,
                            text: reply.text.clone(),
                            reasoning,
                            text_msgs: vec![msgs_of[0].clone()],
                            tool: Some(ToolRun {
                                view,
                                msgs: vec![msgs_of[1].clone()],
                            }),
                            finish: finish.clone(),
                            error: None,
                            verb: None,
                            degraded: false,
                        });
                        continue;
                    }
                }
            }
        }
        // 信封这一线的分派依据：不合法时恰好一条（见 envelope::build_invokes），合法时为空。
        let malformed = reply.tools.first().and_then(|t| t.malformed.clone());
        match malformed.clone() {
            // 信封不合法（缺 name / 混用两种形态 / calls 为空 / 没写完…）：**一个工具都不执行**，
            // 但记一条失败的工具行把"哪里不合法"回注给模型（下一轮自己改）。同样计入上限，不会死循环。
            _ if malformed.is_some() && tools.is_some() => {
                let inv = reply.tools.first().cloned().expect("上臂已判非空");
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                // 回执按判定出的类别给修法（未闭合 / 裸控制字符 / 语法错 / 字段不合法）。
                let mut why = crate::capabilities::llm::api::malformed_report(
                    &ctx.sandbox.texts,
                    inv.malformed.as_ref().expect("上臂已判存在"),
                );
                // 供应商说是长度截断：那"写坏 JSON"就不是模型的错，改法也不同（分次写/拆小步骤）。
                if truncated {
                    why.push('\n');
                    why.push_str(&ctx.sandbox.texts.malformed_truncated);
                }
                let view = ToolCallView {
                    speaker: speaker.to_string(),
                    module: inv.module.clone().unwrap_or_default(),
                    name: inv.name.clone(),
                    ok: false,
                    args: inv.args_json.clone(),
                    output: why,
                    raw: raw.clone(),
                    call_id: String::new(),
                    reply: reply_id,
                };
                on_tool(&view);
                let texts = &ctx.sandbox.texts;
                let msgs_of = reply_msgs(mode, &raw, std::slice::from_ref(&view), texts);
                for m in &msgs_of {
                    msgs.push(m.clone());
                }
                push_round!(Round {
                    reply: reply_id,
                    text: reply.text.clone(),
                    reasoning,
                    text_msgs: vec![msgs_of[0].clone()],
                    tool: Some(ToolRun {
                        view,
                        msgs: vec![msgs_of[1].clone()],
                    }),
                    finish: finish.clone(),
                    error: None,
                    verb: None,
                    degraded: true,
                });
            }
            // 合法信封：**一次回复里的多个调用一起执行**（同一套声明并发调度），各成一条工具行。
            _ if tools.is_some() && !reply.tools.is_empty() => {
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                let invokes = reply.tools.clone();
                // 自由格式工具（patch）只能单发：它的输入是**信封之后的那段正文**（不必转义），
                // 显示正文只认信封**之前**那段——补丁内容不该被当成 AI 发言渲染出来。
                if invokes.len() == 1
                    && crate::capabilities::tools::api::is_freeform(&invokes[0].name)
                {
                    reply.text = invokes[0].lead.clone();
                }
                let plan: Vec<(Option<String>, String, String)> = invokes
                    .iter()
                    .map(|t| {
                        let freeform = crate::capabilities::tools::api::is_freeform(&t.name);
                        let args = if freeform {
                            t.body.clone()
                        } else {
                            t.args_json.clone()
                        };
                        (t.module.clone(), t.name.clone(), args)
                    })
                    .collect();
                // 内置工具（read/write/edit/search）优先且不属于任何模块；外部工具按模块定 cwd。
                note_unauthorized(ctx, &plan, sink);
                let done = run_batch(ctx, &plan);
                // 修过信封就如实标注在回执最前面（模型与用户都能看到核心没有瞎猜）
                let annotate = |outcome: ToolOutcome| -> ToolOutcome {
                    match repaired.as_deref() {
                        Some(what) if !what.is_empty() => ToolOutcome {
                            ok: outcome.ok,
                            output: format!(
                                "{}\n{}",
                                ctx.sandbox.texts.render(
                                    &ctx.sandbox.texts.envelope_repaired,
                                    &[("what", what.to_string())]
                                ),
                                outcome.output
                            ),
                        },
                        _ => outcome,
                    }
                };
                let texts = &ctx.sandbox.texts;
                let views: Vec<ToolCallView> = plan
                    .iter()
                    .enumerate()
                    .map(|(i, (_module, tool, _args))| {
                        let (label, outcome) = done[i].clone();
                        let outcome = annotate(outcome);
                        ToolCallView {
                            speaker: speaker.to_string(),
                            module: label,
                            name: tool.clone(),
                            ok: outcome.ok,
                            args: invokes[i].args_json.clone(),
                            output: outcome.output,
                            raw: raw.clone(),
                            call_id: String::new(),
                            reply: reply_id,
                        }
                    })
                    .collect();
                let msgs_of = reply_msgs(mode, &raw, &views, texts);
                for m in &msgs_of {
                    msgs.push(m.clone());
                }
                // 按原序回填：每条调用一条工具行（多调用时正文只挂第一条）。
                // text = 信封之外的那段正文（可能为空；信封 JSON 已被 parse 剥掉，永不进 text）。
                for (i, view) in views.into_iter().enumerate() {
                    on_tool(&view);
                    push_round!(Round {
                        reply: reply_id,
                        text: if i == 0 {
                            reply.text.clone()
                        } else {
                            String::new()
                        },
                        reasoning: std::mem::take(&mut reasoning),
                        text_msgs: if i == 0 {
                            vec![msgs_of[0].clone()]
                        } else {
                            Vec::new()
                        },
                        tool: Some(ToolRun {
                            view,
                            msgs: vec![msgs_of[i + 1].clone()],
                        }),
                        finish: finish.clone(),
                        error: None,
                        verb: None,
                        degraded: false,
                    });
                }
            }
            // 无工具环境 / 模型不再发起调用：按原文口径如实收录（信封已被剥掉，显示文本里不会有 JSON），循环终止。
            _ => {
                // 只有会出文本行（有正文或思维链）时才往历史里放这条 assistant，
                // 否则实时历史会比重建历史多一条空消息。
                let text = reply.text;
                let has_line = !text.trim().is_empty() || !reasoning.trim().is_empty();
                let text_msgs = if has_line {
                    vec![Msg::assistant(text.trim().to_string())]
                } else {
                    Vec::new()
                };
                push_round!(Round {
                    reply: reply_id,
                    text,
                    reasoning,
                    text_msgs,
                    tool: None,
                    finish,
                    error: None,
                    verb: None,
                    degraded: reply.degraded || reply.verb == Verb::Tool,
                });
                return rounds;
            }
        }
    }
}

// —— 回合驱动：以会话状态跑一次轮循环 ——
//
// **为什么这些方法定义在引擎里**：它们是「驱动」（问模型 → 解析信封 → 调工具 → 落行），
// 会话本身只留状态与簿记。反过来（会话驱动引擎）会形成 `engine ⇄ session` 环。
// 字段以 `pub(super)` 开放：两者同在 `capabilities` 之下（驱动与它会话状态都归会话能力），这是有意的取舍。
// 见 ARCHITECTURE.md §一。

// —— 转录行构造：**行格式只有这一处定义** ——
//
// 定义在引擎里：它由逐轮回调在 `converse_with` 内部调用，那时会话已被拆开；
// 会话只提供 `stream_piece` 与状态。
/// 一轮的转录行：文本行（有正文/思维链时）+ 工具行（有工具时）。
/// **不依赖 `&mut self`**：它由逐轮回调在 `converse_with` 内部调用，那时 `self` 已被拆开。
/// 行号从 `next_line` 递增（回调里记不了账，所以由调用方在回合收尾时按同一批行补 marks）。
/// `turn` = 这一行属于哪个回合（讨论席的回合号）；None = 用该轮自己的回复号（单 agent 每轮各成回合）。
/// **行格式只有这一处定义**：单 agent 与讨论席的行都从这里出（回档按同一口径解析回发言）。
pub fn build_round_lines(
    id: &str,
    texts: &crate::capabilities::prompt::api::ToolTexts,
    round: &Round,
    stopped: bool,
    next_line: &std::cell::Cell<u64>,
    turn: Option<u64>,
) -> Vec<LineView> {
    let text = round.text.trim().to_string();
    let has_line = !text.is_empty() || !round.reasoning.trim().is_empty();
    let truncated = round.truncated();
    let mut reasoning = if round.reasoning.trim().is_empty() {
        None
    } else {
        Some(round.reasoning.clone())
    };
    // 回合号：讨论席一轮一个回合号（整场工作单调递增）；单 agent 的每一轮各成"回合"（回档按它对齐）。
    let turn = turn.unwrap_or(round.reply);
    // 说话人与动词是**结构化字段**（正文里不再带 [谁:动词] 标签）；渲染由 LineView::render 拼回。
    let speaker = id.to_string();
    let verb = round
        .verb
        .map(crate::capabilities::collab::service::round::verb_tag)
        .unwrap_or_default();
    let make = |line: String,
                verb: &str,
                kind: &str,
                reasoning: Option<String>,
                tool: Option<ToolCallView>| {
        let num = next_line.get();
        next_line.set(num + 1);
        LineView {
            id: num,
            reply: round.reply,
            line,
            speaker: speaker.clone(),
            verb: verb.to_string(),
            kind: kind.to_string(),
            reasoning,
            tool,
            degraded: false,
            system: false,
            task: false,
            turn,
        }
    };
    let mut out = Vec::new();
    let text_line = |reasoning: &mut Option<String>, out: &mut Vec<LineView>| {
        // 工具轮没有正文时，思维链必须挂到工具行，不能额外造一条空回答行。
        if !has_line || (text.is_empty() && round.tool.is_some()) {
            return;
        }
        // 正文 = 内容本身（说话人/动词在字段里）；被停/被截断的说明照样跟在正文后。
        let mut line = text.clone();
        if stopped {
            line.push_str(&texts.stopped_suffix);
        }
        if truncated {
            line.push_str(&texts.truncated_suffix);
        }
        out.push(make(line, verb, "msg", reasoning.take(), None));
    };
    match &round.tool {
        Some(run) => {
            // 先出「思考+正文」文本行（只有信封没有正文/思维链时不出空行）。
            text_line(&mut reasoning, &mut out);
            let status = if run.view.ok { "成功" } else { "失败" };
            // 没有文本行时思维链挂到工具行上，不丢。
            // 工具行自带调用视图（呈现层按卡片渲染）：说话人已知，动词留空（不是一次表态）。
            out.push(make(
                format!("工具 {} → {}", run.view.label(), status),
                "",
                "tool",
                reasoning.take(),
                Some(run.view.clone()),
            ));
        }
        None => text_line(&mut reasoning, &mut out),
    }
    out
}
