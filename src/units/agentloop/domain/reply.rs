//! 目的：把一次模型回复与工具结果翻译成发给模型的消息（实时与重建的唯一来源）。
//! 管：`tool_result_body`（一条工具结果的正文）与 `reply_msgs`（回复 + 调用视图 → 消息序列）。
//! 不管：消息的发送（引擎的 `Chat` 端口）；转录行与调用视图的形状（在 transcript）；收据的文字渲染（在 kernel 的 i18n）。
//! 联动：引擎的轮循环与驱动都只经这里拼消息；形状由 ToolMode 决定，切换形态时旧消息自动按新形状表达。

use crate::units::llm::api::Msg;
use crate::units::pure::registry_model::api::ToolMode;
use crate::units::transcript::api::ToolCallView;

/// 目的：一条工具结果消息的**正文**（回放与实时唯一来源；发送视图按它定位读过的那条消息）。
/// 约束：模型通道固定用源语言渲染说明——模型看到的是稳定文本，与界面语言无关。
pub fn tool_result_body(c: &ToolCallView) -> String {
    let body = crate::kernel::api::i18n::render_receipt(
        crate::kernel::api::i18n::Lang::source(),
        &c.output,
        &c.notes,
    );
    format!("[工具结果] {}\n{}", c.label(), body)
}

/// 目的：把**一次模型回复**翻译成发给模型的消息——**实时与重建都只走这一处**。
///   为什么必须只有一处：转录行与消息列表是同一件事的两份表示，两边各拼一次就会漂移：
///   一次回复里有多条原生调用时，实时推的助手消息与重建出来的会不是同一条，
///   第二条起还会漏推助手消息（上下文里因此凭空多/少消息）。
///   形状按**当前形态**决定，所以形态切换时旧消息会被自动表达成新形状（切回去也能还原——事实留在转录里）：
///   - 这条回复的调用都带合法原生 id 且当前走原生通道 → assistant(正文 + tool_calls) + 每条调用一条 role=tool；
///   - 其余（手写信封、原生通道里写坏的调用、切换形态后的旧消息）→ assistant(正文) + 结果当用户消息。
pub fn reply_msgs(mode: ToolMode, raw: &str, calls: &[ToolCallView]) -> Vec<Msg> {
    let protocol = mode == ToolMode::Native
        && !calls.is_empty()
        && calls.iter().all(|c| !c.call_id.is_empty());
    let mut out: Vec<Msg> = Vec::with_capacity(calls.len() + 1);
    if protocol {
        out.push(Msg::assistant_calls(
            raw,
            calls
                .iter()
                .map(|c| crate::units::llm::api::ToolCall {
                    id: c.call_id.clone(),
                    name: c.name.clone(),
                    args_json: c.args.clone(),
                })
                .collect(),
        ));
    } else {
        out.push(Msg::assistant(raw));
    }
    for c in calls {
        let body = tool_result_body(c);
        out.push(if protocol {
            Msg::tool(&c.call_id, body)
        } else {
            Msg::user(body)
        });
    }
    out
}
