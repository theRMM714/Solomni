//! 会话能力的**用例与端口持有者**：历史落盘端口只在这里（R12）。
//!
//! 别的能力要造会话 / 追流水 / 读元信息 / 删会话，走 `api::History`；
//! 呈现层的列表 / 打开 / 删除走 `api::HistoryOps`（由队列代理实现）。
//! 装配（new 出适配器）在组合根；这里只收注入的端口。

use crate::capabilities::session::api::{History, HistoryView, SessionMeta};
use crate::capabilities::session::ports::HistoryStore;
use std::sync::Arc;

/// 会话能力：持历史落盘端口，按用例答话。
pub struct SessionService {
    store: Arc<dyn HistoryStore + Send + Sync>,
}

impl SessionService {
    /// 组合根专用。
    pub fn new(store: Arc<dyn HistoryStore + Send + Sync>) -> SessionService {
        SessionService { store }
    }
}

impl History for SessionService {
    fn create(&self, meta: &SessionMeta) -> Result<(), String> {
        self.store.create(meta)
    }

    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        self.store.save_meta(meta)
    }

    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.store.append(name, events)
    }

    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.store.list()
    }

    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.store.load(name)
    }

    fn delete(&self, name: &str) -> Result<bool, String> {
        self.store.delete(name)
    }
}

// ---------- 核心操作回路（核心 AI 的一次操作） ----------
//
// 原料全是会话自己的东西：只读核实要经 `MemberTools`（工具面 + 观察账本 + 围栏），
// 事实外送用的是会话的行格式（`SessionEvent` / `LineView` / `ToolCallView`），
// 正文分片用的是 `stream_piece`。所以它是"在会话的工具面上跑一次核心操作并产出会话行"的用例，
// 归会话能力；**调用侧的取消/分片包装也只有这一处**（协作会话、核心推荐、单 agent 都走它）。
// 语义见 docs/tools/tools-and-roles.md（核心操作必须走工具调用 + 只读核实回路）。

/// 原文前 n 个字符（如实报错时带上一点现场；按字符切，不切坏多字节）。
fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 核心这一轮的**行**：工具行（谁=核心、动词=工具名、带调用视图）+ 正文/思维链行。
/// 核心操作**没有 agent 会话**，所以这些行必须推给调用方（协作会话 / 系统会话）：
/// 落不落盘由那个会话模块自己定（协作要留档，系统会话只推不留）。
fn core_rows(
    tool: &str,
    view: crate::capabilities::session::api::ToolCallView,
    reasoning: &str,
    text: &str,
) -> Vec<crate::capabilities::session::api::LineView> {
    let mut rows = Vec::new();
    if !text.trim().is_empty() {
        rows.push(crate::capabilities::session::api::LineView {
            speaker: "核心".to_string(),
            verb: String::new(),
            kind: "msg".to_string(),
            line: text.trim().to_string(),
            ..Default::default()
        });
    }
    rows.push(crate::capabilities::session::api::LineView {
        speaker: "核心".to_string(),
        verb: tool.to_string(),
        kind: "tool".to_string(),
        line: format!(
            "工具 {} → {}",
            view.label(),
            if view.ok { "成功" } else { "失败" }
        ),
        reasoning: if reasoning.trim().is_empty() {
            None
        } else {
            Some(reasoning.to_string())
        },
        tool: Some(view),
        ..Default::default()
    });
    rows
}

/// 把**一次模型回复**翻译成发给模型的消息——**实时与重建都只走这一处**。
///
/// 为什么必须只有一处：转录行与消息列表是同一件事的两份表示，两边各拼一次就会漂移。
/// 真实缺陷就出在这里：一次回复里有多条原生调用时，实时推的助手消息与重建出来的既不是同一条，
/// 第二条起实时还根本不推助手消息（上下文里因此凭空多/少消息）。
///
/// 形状按**当前形态**决定，所以形态切换时旧消息会被自动表达成新形状（切回去也能还原——事实留在转录里）：
/// - 这条回复的调用都带合法原生 id 且当前走原生通道 → assistant(正文 + tool_calls) + 每条调用一条 role=tool；
/// - 其余（手写信封、原生通道里写坏的调用、切换形态后的旧消息）→ assistant(正文) + 结果当用户消息。
pub fn reply_msgs(
    mode: crate::capabilities::llm::api::ToolMode,
    raw: &str,
    calls: &[crate::capabilities::session::api::ToolCallView],
    texts: &crate::capabilities::prompt::api::ToolTexts,
) -> Vec<crate::capabilities::llm::api::Msg> {
    let protocol = mode == crate::capabilities::llm::api::ToolMode::Native
        && !calls.is_empty()
        && calls.iter().all(|c| !c.call_id.is_empty());
    let mut out: Vec<crate::capabilities::llm::api::Msg> = Vec::with_capacity(calls.len() + 1);
    if protocol {
        out.push(crate::capabilities::llm::api::Msg::assistant_calls(
            raw,
            calls
                .iter()
                .map(|c| crate::capabilities::llm::api::ToolCall {
                    id: c.call_id.clone(),
                    name: c.name.clone(),
                    args_json: c.args.clone(),
                })
                .collect(),
        ));
    } else {
        out.push(crate::capabilities::llm::api::Msg::assistant(raw));
    }
    for c in calls {
        let body = texts.render(
            &texts.tool_result_wrapper,
            &[("label", c.label()), ("output", c.output.clone())],
        );
        out.push(if protocol {
            crate::capabilities::llm::api::Msg::tool(&c.call_id, body)
        } else {
            crate::capabilities::llm::api::Msg::user(body)
        });
    }
    out
}

/// 核心 AI 的一次**操作**：声明该角色的工具面、跑一次模型、从**工具调用参数**里取载荷。
///
/// 为什么必须走工具调用（见 docs/tools/tools-and-roles.md）：核心操作会驱动核心走下一步
/// （建任务链、判交付、确认名单、推进状态机），属于"操作"而不是"说话"——
/// 正文里手写 JSON 既没有 schema 校验、也不进工具台账，写坏就整轮失败。
///
/// 两条通道都认：native 从结构化槽位取；手写信封从正文里的信封取。
/// 取消由调用方给的标志说了算（`None` = 这一趟不听取消）；**分片与取消的包装只有这一处**，
/// 调用方不再各自拼一遍 `keep` 闭包。
// 与 turn_with / converse_with 同一组参数（工具面 / 通道 / 消息 / 出口）：不是随手堆参数，
// 收口成参数对象只会把参数挪个地方、并让"谁拿到什么"更难读。有意取舍（见 docs/testing/quality-isolation.md）。
#[allow(clippy::too_many_arguments)]
pub fn core_operation(
    systools: &dyn crate::capabilities::tools::api::Tools,
    role: &str,
    tool: &str,
    mode: crate::capabilities::llm::api::ToolMode,
    chat: &mut dyn crate::capabilities::llm::api::Chat,
    msgs: &[crate::capabilities::llm::api::Msg],
    opts: crate::capabilities::llm::api::CompleteOpts<'static>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    verify: Option<&mut crate::capabilities::session::api::MemberTools>,
    // 核心这一轮的**事实出口**（推；落盘与否由会话模块决定）。
    sink: &mut dyn FnMut(crate::capabilities::session::api::SessionEvent),
) -> Result<serde_json::Value, String> {
    // 声明面按**通道形态**给：原生通道才声明（信封通道的模型看提示词里的工具说明）。
    let face_rows: Vec<(&str, &crate::capabilities::tools::api::ToolSchema)> =
        systools.tool_face(role).unwrap_or_default();
    let face_ids: Vec<String> = face_rows.iter().map(|(id, _)| id.to_string()).collect();
    let mut opts = opts;
    let decls: Vec<crate::capabilities::llm::api::ToolDecl> =
        if mode == crate::capabilities::llm::api::ToolMode::Native {
            face_rows.iter().map(|(id, s)| s.decl(id)).collect()
        } else {
            Vec::new()
        };
    if !decls.is_empty() {
        opts.tools = Some(&decls);
    }
    // **核实回路**：核心操作也是"先看现场再下结论"。模型想先读/查（很合理的动作）时，
    // 执行它请求的**只读**工具并把结果回灌，然后再要那一次核心操作调用。
    // 没有这条，模型一想核实就被判"没调用 X"→整步中断（真机上就是这么卡死的）。
    let mut msgs = msgs.to_vec();
    let mut verify = verify;
    // **核心的正文/思维链也逐片上屏**：与成员、单 agent 同一条规则（信封之前照常外送，见 session::api::stream_piece），
    // 不再整块蹦出来。取消仍由调用方的标志说了算——这里只是把它包一层，顺手把片段推出去。
    let mut acc = String::new();
    loop {
        let done = {
            let mut on = |chunk: crate::capabilities::llm::api::Chunk| {
                let mut kind = "text";
                let mut piece = String::new();
                match &chunk {
                    crate::capabilities::llm::api::Chunk::Start => {
                        acc.clear();
                        kind = "start";
                    }
                    crate::capabilities::llm::api::Chunk::Text(t) => {
                        let (send, next) = crate::capabilities::session::api::stream_piece(&acc, t);
                        piece = send;
                        acc = next;
                    }
                    crate::capabilities::llm::api::Chunk::Reasoning(r) => {
                        kind = "reasoning";
                        piece = r.clone();
                    }
                }
                sink(crate::capabilities::session::api::SessionEvent::Delta {
                    speaker: "核心".to_string(),
                    kind: kind.to_string(),
                    text: piece,
                });
                match cancel {
                    Some(c) => !c.load(std::sync::atomic::Ordering::Relaxed),
                    None => true,
                }
            };
            chat.complete(&msgs, opts, &mut on)
        };
        if let Some(err) = done.error {
            return Err(err);
        }
        let parsed = crate::capabilities::llm::api::parse(&done.raw);
        // native：结构化槽位里找这个名字的调用。
        if let Some(c) = done.calls.iter().find(|c| c.name == tool) {
            let payload: serde_json::Value = serde_json::from_str(&c.args_json)
                .map_err(|e| format!("{} 的参数不是合法 JSON（{}）：{}", tool, e, c.args_json))?;
            // **推这一轮的事实**：它调了什么、带了什么参数、模型怎么想的。
            for row in core_rows(
                tool,
                crate::capabilities::session::api::ToolCallView {
                    speaker: "核心".to_string(),
                    module: String::new(),
                    name: tool.to_string(),
                    ok: true,
                    args: c.args_json.clone(),
                    output: head_chars(&payload.to_string(), 400),
                    raw: done.raw.clone(),
                    call_id: c.id.clone(),
                    reply: 0,
                },
                &done.reasoning,
                &parsed.text,
            ) {
                sink(crate::capabilities::session::api::SessionEvent::Transcript(
                    vec![row],
                ));
            }
            return Ok(payload);
        }
        // 手写信封：正文里的信封里找。
        if let Some(t) = parsed.tools.iter().find(|t| t.name == tool) {
            let payload: serde_json::Value = serde_json::from_str(&t.args_json)
                .map_err(|e| format!("{} 的参数不是合法 JSON（{}）：{}", tool, e, t.args_json))?;
            for row in core_rows(
                tool,
                crate::capabilities::session::api::ToolCallView {
                    speaker: "核心".to_string(),
                    module: String::new(),
                    name: tool.to_string(),
                    ok: true,
                    args: t.args_json.clone(),
                    output: head_chars(&payload.to_string(), 400),
                    raw: done.raw.clone(),
                    call_id: String::new(),
                    reply: 0,
                },
                &done.reasoning,
                &parsed.text,
            ) {
                sink(crate::capabilities::session::api::SessionEvent::Transcript(
                    vec![row],
                ));
            }
            return Ok(payload);
        }
        // 没有目标调用：看它请求的是不是**该角色拿得到的只读核实工具**（read / search）。
        let calls: Vec<(String, String, String)> =
            if mode == crate::capabilities::llm::api::ToolMode::Native && !done.calls.is_empty() {
                done.calls
                    .iter()
                    .map(|c| (c.id.clone(), c.name.clone(), c.args_json.clone()))
                    .collect()
            } else {
                parsed
                    .tools
                    .iter()
                    .map(|t| (String::new(), t.name.clone(), t.args_json.clone()))
                    .collect()
            };
        let ctx = match verify.as_deref_mut() {
            Some(c) => c,
            None => {
                return Err(format!(
                    "没有调用 {}（核心操作必须走工具调用）：{}",
                    tool,
                    head_chars(&done.raw, 200)
                ))
            }
        };
        let readonly: Vec<(String, String, String)> = calls
            .into_iter()
            .filter(|(_, n, _)| {
                face_ids.iter().any(|f| f == n)
                    && ctx
                        .builtin_tools
                        .get(n)
                        .map(|s| s.capability == "fs-read")
                        .unwrap_or(false)
            })
            .collect();
        if readonly.is_empty() {
            return Err(format!(
                "没有调用 {}（核心操作必须走工具调用）：{}",
                tool,
                head_chars(&done.raw, 200)
            ));
        }
        let mut views: Vec<crate::capabilities::session::api::ToolCallView> = Vec::new();
        for (call_id, name, args) in readonly {
            let out = ctx.tools.run_builtin(
                &ctx.sandbox,
                &ctx.builtin_tools,
                &mut ctx.observations,
                &name,
                &args,
            );
            let view = crate::capabilities::session::api::ToolCallView {
                speaker: "核心".to_string(),
                module: String::new(),
                name,
                ok: out.ok,
                args,
                output: out.output,
                raw: done.raw.clone(),
                call_id,
                reply: 0,
            };
            // 核心"先核实"这一步也如实推出去（用户看得到它在读什么、查什么）。
            for row in core_rows(&view.name, view.clone(), "", "") {
                sink(crate::capabilities::session::api::SessionEvent::Transcript(
                    vec![row],
                ));
            }
            views.push(view);
        }
        // 按通道形态把结果回灌（原生：一条助手消息带 tool_calls + 每条结果 role=tool）。
        for m in reply_msgs(mode, &done.raw, &views, &ctx.sandbox.texts) {
            msgs.push(m);
        }
    }
}
