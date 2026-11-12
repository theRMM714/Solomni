//! 目的：事件流视图助手——把 Conductor 吐出的事件流还原成「用户看到的那几行」。
//! 管：裁决卡回答、转录行/工具行的视图与文本、某段文字的在场与否。
//! 不管：造会话与名单（`session.rs`）、通道替身（`chat.rs`）。
//! 联动：`src/capabilities/conductor/api.rs` 的 `SessionEvent` 与呈现层的 `LineView`。

use super::*;
use super::super::prelude::*;

/// 目的：测试里**回答当前那张裁决卡**（卡号从会话的挂起取）：返回这一答产生的事件。
/// 约束：与呈现层同一条路——带选项 id（+ 附言），由 `collab_answer` 校验与落档。
pub(crate) fn answer_card(
    core: &mut Conductor,
    sid: &str,
    option: &str,
    note: &str,
) -> Result<Vec<SessionEvent>, String> {
    let queue = core.open_queue(sid)?.ok_or("现在没有挂着的裁决卡")?;
    core.collab_answer(sid, &queue.card.id, option, note)
}

/// 目的：从事件流里抽出转录行并**渲染成文本**（行怎么变文本只有 LineView::render 一处）。
pub(crate) fn replay_lines(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .flat_map(|e| {
            e.get("lines")
                .and_then(|l| l.as_array())
                .cloned()
                .unwrap_or_default()
        })
        .map(|l| {
            serde_json::from_value::<crate::capabilities::session::api::LineView>(l)
                .map(|v| v.render())
        })
        .filter_map(Result::ok)
        .collect()
}

/// 目的：一批事件里**用户看得到的说明文本**——瞬时 Notice 与已行化的 Transcript 正文都算。
/// 约束：只看呈现文本，不判事件种类（系统提示行化后两者在界面是同一块）。
pub(crate) fn shown_texts(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Notice(n) => vec![n.clone()],
            SessionEvent::Transcript(ls) => ls.iter().map(|l| l.render()).collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// 目的：这行事件里有没有一条"用户看得到的说明"包含 needle。
pub(crate) fn shown_contains(events: &[SessionEvent], needle: &str) -> bool {
    shown_texts(events).iter().any(|t| t.contains(needle))
}

/// 目的：包含 needle 的那条事件在流里的位置（一条事件里多行只算一次）。
pub(crate) fn shown_at(events: &[SessionEvent], needle: &str) -> Option<usize> {
    events.iter().position(|e| match e {
        SessionEvent::Notice(n) => n.contains(needle),
        SessionEvent::Transcript(ls) => ls.iter().any(|l| l.render().contains(needle)),
        _ => false,
    })
}

/// 目的：事件流里的转录行视图：(id, line, 是否是 tool 行)。
pub(crate) fn transcript_rows(events: &[SessionEvent]) -> Vec<(u64, String, bool)> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .map(|l| (l.id, l.render(), l.tool.is_some()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 目的：事件流里的工具调用视图（tool 行携带的那个）。
pub(crate) fn tool_views(
    events: &[SessionEvent],
) -> Vec<crate::capabilities::session::api::ToolCallView> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => {
                ls.iter().filter_map(|l| l.tool.clone()).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect()
}

/// 目的：转录里所有非空的行级原文（每条回复应只有一份）。
pub(crate) fn transcript_raws(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .filter(|l| !l.raw.is_empty())
                .map(|l| l.raw.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 目的：事件流里 tool 行的行文本（给人看的那一行）。
pub(crate) fn tool_line_texts(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .filter(|l| l.tool.is_some())
                .map(|l| l.render())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}
