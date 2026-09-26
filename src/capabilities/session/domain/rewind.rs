//! 回档的**行 / 事件算术**：纯函数，不碰 IO，也不碰会话实例。
//!
//! **为什么归 session 而不是独立能力**：它们读写的全部状态（转录行、`marks` / `line_reply` /
//! `next_line`）都归会话所有，它们自己没有状态——不满足「独立状态所有权」这条必要判据。
//! `Core` 里的回档**编排**（撤子会话、截事件流水、重建会话）是门面职责，留在 `core`。
//! 见 docs/architecture/refactor-plan.md §4.2 批次 13。

/// 主会话第 keep_id 行所属的**回合**（没有 = 0）。
pub fn turn_of_line(events: &[serde_json::Value], keep_id: u64) -> u64 {
    for ev in events {
        if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
            for l in lines {
                if l.get("id").and_then(|i| i.as_u64()) == Some(keep_id) {
                    return l.get("turn").and_then(|t| t.as_u64()).unwrap_or(0);
                }
            }
        }
    }
    0
}

/// 最后一个 `turn ≤ keep_turn` 的行的 id（没有 = 0 = 该会话全部截掉）。
pub fn last_line_within(events: &[serde_json::Value], keep_turn: u64) -> u64 {
    // 规则：**从第一行超出保留点的行开始全截掉**（turn = 0 的行不属于任何回合，
    // 跟着它前面那一回合走——用户插话、需求这类行因此不会被误删）。
    let mut cut = 0u64;
    for ev in events {
        if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
            for l in lines {
                let t = l.get("turn").and_then(|t| t.as_u64()).unwrap_or(0);
                if t > keep_turn {
                    return l.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                }
                cut = l.get("id").and_then(|i| i.as_u64()).unwrap_or(cut) + 1;
            }
        }
    }
    cut
}

/// 应用流水里的 rewind 记录：会话内容 = 回放时只保留「id < keep」的转录行（回档 = 删该行及其后）。
pub fn truncate_events(events: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut content: Vec<serde_json::Value> = Vec::new();
    for ev in events {
        match ev.get("type").and_then(|t| t.as_str()) {
            Some("rewind") => {
                // 缺 keep 字段 = 不截断（安全默认：宁可多留，不可清空一切）。
                if let Some(keep) = ev.get("keep").and_then(|k| k.as_u64()) {
                    content = cut_before_line(&content, keep);
                }
            }
            _ => content.push(ev.clone()),
        }
    }
    content
}

/// 只保留「转录行 id < keep」的行（回档 = 删除该行及其后；keep = 0 → 转录清空）。
/// 一旦某条事件里的行被截掉，其后的事件一并丢弃（事件流是时序的）。
pub fn cut_before_line(events: &[serde_json::Value], keep: u64) -> Vec<serde_json::Value> {
    let keep = align_keep(events, keep);
    let mut out = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) == Some("transcript") {
            if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
                let kept: Vec<serde_json::Value> = lines
                    .iter()
                    .take_while(|l| {
                        l.get("id")
                            .and_then(|i| i.as_u64())
                            .map(|i| i < keep)
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                let done = kept.len() < lines.len();
                if !kept.is_empty() {
                    let mut e2 = ev.clone();
                    e2["lines"] = serde_json::Value::Array(kept);
                    out.push(e2);
                }
                if done {
                    return out;
                }
            }
        } else {
            out.push(ev.clone());
        }
    }
    out
}

/// 把"保留 id < keep"对齐到**回复边界**（见 crate::capabilities::session::api::keep_whole_replies）：
/// keep 落在某次回复内部时退到该回复第一行之前，返回新的 keep（没有这样的行 = u64::MAX，即不截）。
pub fn align_keep(events: &[serde_json::Value], keep: u64) -> u64 {
    let rows: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flatten()
        .collect();
    let idx = rows
        .iter()
        .position(|l| {
            l.get("id")
                .and_then(|i| i.as_u64())
                .map(|i| i >= keep)
                .unwrap_or(false)
        })
        .unwrap_or(rows.len());
    let replies: Vec<u64> = rows.iter().map(|l| line_reply_of(l)).collect();
    let aligned = crate::capabilities::session::api::keep_whole_replies(&replies, idx);
    rows.get(aligned)
        .and_then(|l| l.get("id").and_then(|i| i.as_u64()))
        .unwrap_or(u64::MAX)
}

/// 一行属于哪次回复：工具行的号在调用视图里，其余行在 LineView 上。
/// 号 = 0 视为"没写"（回复号从 1 起），此时用**该行自己的 id**——绝不把相邻行误并成一组。
pub fn line_reply_of(l: &serde_json::Value) -> u64 {
    let own = l.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
    let stored = match l.get("tool") {
        Some(t) => t.get("reply").and_then(|x| x.as_u64()),
        None => l.get("reply").and_then(|x| x.as_u64()),
    };
    stored.filter(|r| *r != 0).unwrap_or(own)
}

/// 找最后一条满足条件的转录行的 id（按**结构化字段**判，不匹配正文）。
pub fn find_line_id(
    events: &[serde_json::Value],
    pick: impl Fn(&crate::capabilities::session::api::LineView) -> bool,
) -> Option<u64> {
    let mut found = None;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            if let Ok(v) =
                serde_json::from_value::<crate::capabilities::session::api::LineView>(l.clone())
            {
                if pick(&v) {
                    found = Some(v.id);
                }
            }
        }
    }
    found
}
