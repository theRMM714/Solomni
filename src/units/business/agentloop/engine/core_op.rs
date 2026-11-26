//! 目的：核心操作回路——在工具面上跑一次"核心 AI 的操作"（取载荷、走只读核实、把事实推给调用方）。
//! 管：`core_operation` 与它的行构造（`core_rows` / `head_chars`）：取消与分片的包装只有这一处。
//! 不管：会话对象与成员回合（`domain` 与同层的 driver/round）；工具的声明与执行（tools 端口后）；落盘（会话单元）。
//! 联动：会话单元的用例驱动它；语义见 docs/tools/tools-and-roles.md（核心操作必须走工具调用 + 只读核实回路）。

use crate::units::business::agentloop::domain::member_tools::CoreVerify;
use crate::units::business::agentloop::domain::reply::reply_msgs;
use crate::units::business::agentloop::domain::session::{delta_visible, stream_piece};
use crate::units::business::session_model::api as session_model;

// 核心操作回路：在会话的工具面上跑一次核心操作并产出会话行；调用侧的取消 / 分片包装只有这一处。
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
    view: crate::units::business::transcript::api::ToolCallView,
    raw: &str,
    reasoning: &str,
    text: &str,
) -> Vec<crate::units::business::transcript::api::LineView> {
    let mut rows = Vec::new();
    if !text.trim().is_empty() {
        rows.push(crate::units::business::transcript::api::LineView {
            speaker: "核心".to_string(),
            verb: String::new(),
            kind: "msg".to_string(),
            line: text.trim().to_string(),
            ..Default::default()
        });
    }
    rows.push(crate::units::business::transcript::api::LineView {
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
        raw: raw.to_string(),
        tool: Some(view),
        ..Default::default()
    });
    rows
}

/// 核心 AI 的一次**操作**：声明该席位的工具面、跑一次模型、从**工具调用参数**里取载荷。
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
    systools: &dyn crate::units::business::tools::api::Tools,
    seat: &str,
    tool: &str,
    mode: crate::units::pure::registry_model::api::ToolMode,
    chat: &mut dyn crate::units::business::llm::api::Chat,
    msgs: &[crate::units::business::llm::api::Msg],
    opts: crate::units::business::llm::api::CompleteOpts<'static>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    verify: Option<&mut (dyn CoreVerify + 'static)>,
    // 核心这一轮的**事实出口**（推；落盘与否由会话模块决定）。
    sink: &mut dyn FnMut(crate::units::business::transcript::api::SessionEvent),
) -> Result<serde_json::Value, String> {
    // 声明面按**通道形态**给：原生通道才声明（信封通道的模型看提示词里的工具说明）。
    // 席位面由机制声明按总表合成；席位没登记时退化成空面（装配期已由 `mechanism::problems` 拦下）。
    let book = systools.book();
    // 这一轮的面 = 席位自己声明的工具（loop 来源）∪ 只读核实面（read / list / grep）。
    // 核实面恒在：核心操作也要先看现场再下结论，而 `is_readonly` 只放行这三个。
    let mut ids: Vec<&str> = match session_model::seat(seat) {
        Some(s) => s.tools.to_vec(),
        None => Vec::new(),
    };
    for id in [
        crate::units::business::tools::api::READ,
        crate::units::business::tools::api::LIST,
        crate::units::business::tools::api::GREP,
    ] {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let face_rows: Vec<(&str, &crate::units::business::tools::api::ToolSchema)> = ids
        .into_iter()
        .filter_map(|id| book.get(id).map(|s| (id, s)))
        .collect();
    let face_ids: Vec<String> = face_rows.iter().map(|(id, _)| id.to_string()).collect();
    // 声明面先备好：它被 opts 借走，**必须先于 opts 声明**（否则借用活得比它长）。
    let decls: Vec<crate::units::business::llm::api::ToolDecl> =
        if mode == crate::units::pure::registry_model::api::ToolMode::Native {
            face_rows.iter().map(|(id, s)| s.decl(id)).collect()
        } else {
            Vec::new()
        };
    let mut opts = opts;
    if let Some(c) = cancel {
        opts.cancel = Some(crate::units::business::llm::api::cancel_flag(
            std::sync::Arc::clone(c),
        ));
    }
    if !decls.is_empty() {
        opts.tools = Some(&decls);
    }
    // **核实回路**：核心操作也先看现场再下结论——模型想先读 / 查时，执行它请求的**只读**工具并回灌结果，再要核心操作调用。
    // 没有这条，模型一想核实就被判"没调用 X"→整步中断（真机上就是这么卡死的）。
    let mut msgs = msgs.to_vec();
    let mut verify = verify;
    // **核心的正文/思维链也逐片上屏**：与成员、单 agent 同一条规则（信封之前照常外送，见 session::api::stream_piece），
    // 不再整块蹦出来。取消仍由调用方的标志说了算——这里只是把它包一层，顺手把片段推出去。
    let mut acc = String::new();
    loop {
        let done = {
            let mut on = |chunk: crate::units::business::llm::api::Chunk| {
                let mut kind = "text";
                let mut piece = String::new();
                match &chunk {
                    crate::units::business::llm::api::Chunk::Start => {
                        acc.clear();
                        kind = "start";
                    }
                    crate::units::business::llm::api::Chunk::Text(t) => {
                        let (send, next) = stream_piece(&acc, t);
                        piece = send;
                        acc = next;
                    }
                    crate::units::business::llm::api::Chunk::Reasoning(r) => {
                        kind = "reasoning";
                        piece = r.clone();
                    }
                }
                // 空片段不刷事件流（信封被抑制 / 重复快照合并掉的片都不外送）。
                if delta_visible(kind, &piece) {
                    sink(
                        crate::units::business::transcript::api::SessionEvent::Delta {
                            speaker: "核心".to_string(),
                            kind: kind.to_string(),
                            text: piece,
                        },
                    );
                }
                match cancel {
                    Some(c) => !c.load(std::sync::atomic::Ordering::Relaxed),
                    None => true,
                }
            };
            chat.complete(&msgs, opts.clone(), &mut on)
        };
        // 停止优先：已取消的调用不得再解析载荷、执行核实工具或进入下一轮。
        if done.cancelled || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err("已停止".to_string());
        }
        if let Some(err) = done.error {
            return Err(err);
        }
        let parsed = crate::units::business::llm::api::parse(&done.raw);
        // native：结构化槽位里找这个名字的调用。
        if let Some(c) = done.calls.iter().find(|c| c.name == tool) {
            let payload: serde_json::Value = serde_json::from_str(&c.args_json)
                .map_err(|e| format!("{} 的参数不是合法 JSON（{}）：{}", tool, e, c.args_json))?;
            // **推这一轮的事实**：它调了什么、带了什么参数、模型怎么想的。
            for row in core_rows(
                tool,
                crate::units::business::transcript::api::ToolCallView {
                    speaker: "核心".to_string(),
                    module: String::new(),
                    name: tool.to_string(),
                    ok: true,
                    severity: crate::kernel::api::note::Severity::Info,
                    notes: Vec::new(),
                    args: c.args_json.clone(),
                    output: head_chars(&payload.to_string(), 400),
                    call_id: c.id.clone(),
                    reply: 0,
                    observations: Vec::new(),
                    shell: None,
                },
                &done.raw,
                &done.reasoning,
                &parsed.text,
            ) {
                sink(crate::units::business::transcript::api::SessionEvent::Transcript(vec![row]));
            }
            return Ok(payload);
        }
        // 手写信封：正文里的信封里找。
        if let Some(t) = parsed.tools.iter().find(|t| t.name == tool) {
            let payload: serde_json::Value = serde_json::from_str(&t.args_json)
                .map_err(|e| format!("{} 的参数不是合法 JSON（{}）：{}", tool, e, t.args_json))?;
            for row in core_rows(
                tool,
                crate::units::business::transcript::api::ToolCallView {
                    speaker: "核心".to_string(),
                    module: String::new(),
                    name: tool.to_string(),
                    ok: true,
                    severity: crate::kernel::api::note::Severity::Info,
                    notes: Vec::new(),
                    args: t.args_json.clone(),
                    output: head_chars(&payload.to_string(), 400),
                    call_id: String::new(),
                    reply: 0,
                    observations: Vec::new(),
                    shell: None,
                },
                &done.raw,
                &done.reasoning,
                &parsed.text,
            ) {
                sink(crate::units::business::transcript::api::SessionEvent::Transcript(vec![row]));
            }
            return Ok(payload);
        }
        // 没有目标调用：看它请求的是不是**该席位拿得到的只读核实工具**（read / grep）。
        let calls: Vec<(String, String, String)> = if mode
            == crate::units::pure::registry_model::api::ToolMode::Native
            && !done.calls.is_empty()
        {
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
                ));
            }
        };
        let readonly: Vec<(String, String, String)> = calls
            .into_iter()
            .filter(|(_, n, _)| face_ids.iter().any(|f| f == n) && ctx.is_readonly(n))
            .collect();
        if readonly.is_empty() {
            return Err(format!(
                "没有调用 {}（核心操作必须走工具调用）：{}",
                tool,
                head_chars(&done.raw, 200)
            ));
        }
        let mut views: Vec<crate::units::business::transcript::api::ToolCallView> = Vec::new();
        for (call_id, name, args) in readonly {
            let out = ctx.run_readonly(&name, &args);
            let view = crate::units::business::transcript::api::ToolCallView {
                speaker: "核心".to_string(),
                module: String::new(),
                name,
                ok: out.ok,
                severity: out.severity,
                notes: out.notes.clone(),
                args,
                output: out.output,
                call_id,
                reply: 0,
                observations: Vec::new(),
                shell: None,
            };
            // 核心"先核实"这一步也如实推出去（用户看得到它在读什么、查什么）。
            for row in core_rows(&view.name, view.clone(), &done.raw, "", "") {
                sink(crate::units::business::transcript::api::SessionEvent::Transcript(vec![row]));
            }
            views.push(view);
        }
        // 按通道形态把结果回灌（原生：一条助手消息带 tool_calls + 每条结果 role=tool）。
        for m in reply_msgs(mode, &done.raw, &views) {
            msgs.push(m);
        }
    }
}
