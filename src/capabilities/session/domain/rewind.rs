//! 回档的**行 / 事件算术**：纯函数，不碰 IO，也不碰会话实例。
//!
//! **为什么归 session 而不是独立能力**：它们读写的全部状态（转录行、`marks` / `line_reply` /
//! `next_line`）都归会话所有，它们自己没有状态——不满足「独立状态所有权」这条必要判据。
//! `Conductor` 里的回档**编排**（撤子会话、截事件流水、重建会话）是门面职责，留在 `conductor`。
//! 见 ARCHITECTURE.md §六。

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

/// 回档模式：**留档**（标记 + 折叠，可恢复）或**删除**（真的截断，不可恢复）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindMode {
    Archive,
    Delete,
    /// 仅用于子会话同步的"恢复"动作（不作为标记写入流水）。
    Restore,
}

/// 流水里一条回档标记（只追加；删除模式会重写文件，因此新数据里通常只有留档标记）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindMark {
    /// 标记的单调 id（呈现与恢复按它定位）。
    pub mark: u64,
    pub mode: RewindMode,
    /// 保留 id < before 的行。
    pub before: u64,
    /// 标记之后新行从此起的 id（留档用；删除模式等于 before）。
    pub from: u64,
    /// 标记点对应的共享区提交（None = 空共享区）。
    pub commit: Option<u64>,
    /// 标记之前的共享区头（恢复时回到它）。
    pub after: Option<u64>,
}

impl RewindMark {
    /// 从一条流水事件解析；不是回档记录 = None。
    /// 旧格式（只有 keep、没有 mode/from）按删除读：before = from = keep。
    pub fn from_value(ev: &serde_json::Value) -> Option<RewindMark> {
        if ev.get("type").and_then(|t| t.as_str()) != Some("rewind") {
            return None;
        }
        let before = ev
            .get("before")
            .and_then(|v| v.as_u64())
            .or_else(|| ev.get("keep").and_then(|v| v.as_u64()))?;
        let mode = match ev.get("mode").and_then(|m| m.as_str()) {
            Some("archive") => RewindMode::Archive,
            _ => RewindMode::Delete,
        };
        Some(RewindMark {
            mark: ev.get("mark").and_then(|v| v.as_u64()).unwrap_or(0),
            mode,
            before,
            from: ev.get("from").and_then(|v| v.as_u64()).unwrap_or(before),
            commit: ev.get("commit").and_then(|v| v.as_u64()),
            after: ev.get("after").and_then(|v| v.as_u64()),
        })
    }
}

/// 流水里全部回档标记（按出现顺序）。恢复 = 在该标记处截断文件，所以更晚的标记随之一并消失。
pub fn rewind_marks(events: &[serde_json::Value]) -> Vec<RewindMark> {
    events.iter().filter_map(RewindMark::from_value).collect()
}

/// 一行是否被某个**留档**标记折叠（落在它的 [before, from) 内）。
pub fn folded(marks: &[RewindMark], id: u64) -> bool {
    marks
        .iter()
        .any(|m| m.mode == RewindMode::Archive && m.before <= id && id < m.from)
}

/// 应用回档标记后的**活动视图**：
/// - 留档折叠的行不进内容（模型与界面都看不到；文件里仍在，恢复后回来）；
/// - 旧格式的删除标记按位置截断它后面的内容；
/// - 回档标记本身作为结构化事件保留（呈现层据此画留档分界），模型侧会忽略它。
pub fn truncate_events(events: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let marks = rewind_marks(events);
    let mut content: Vec<serde_json::Value> = Vec::new();
    for ev in events {
        match ev.get("type").and_then(|t| t.as_str()) {
            Some("transcript") => {
                let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
                    continue;
                };
                let kept: Vec<serde_json::Value> = lines
                    .iter()
                    .filter(|l| {
                        let id = l.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                        !folded(&marks, id)
                    })
                    .cloned()
                    .collect();
                if !kept.is_empty() {
                    let mut e2 = ev.clone();
                    e2["lines"] = serde_json::Value::Array(kept);
                    content.push(e2);
                }
            }
            Some("rewind") => {
                // 旧格式（删除）标记：按位置截断它后面的内容（新格式删除会重写文件，不留标记）。
                if let Some(m) = RewindMark::from_value(ev) {
                    if m.mode == RewindMode::Delete {
                        content = cut_before_line(&content, m.before);
                    }
                }
                content.push(ev.clone());
            }
            _ => content.push(ev.clone()),
        }
    }
    content
}

/// 流水里用过的**最大行 id + 1**：新行从这里续号，绝不复用被折叠行的 id。
pub fn next_line_id(events: &[serde_json::Value]) -> u64 {
    let mut next = 0u64;
    for ev in events {
        if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
            for l in lines {
                if let Some(id) = l.get("id").and_then(|i| i.as_u64()) {
                    next = next.max(id + 1);
                }
            }
        }
    }
    next
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

/// 转录流水里用过的最大回复 id：重建时据此续号。
/// 为什么必须续号：回复 id 是分组依据，重复就会把新回复与旧回复并成一组。
pub fn max_reply(events: &[serde_json::Value]) -> u64 {
    let mut max = 0u64;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            if let Some(r) = l.get("reply").and_then(|x| x.as_u64()) {
                max = max.max(r);
            }
            if let Some(r) = l
                .get("tool")
                .and_then(|t| t.get("reply"))
                .and_then(|x| x.as_u64())
            {
                max = max.max(r);
            }
        }
    }
    max
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(events: &[serde_json::Value]) -> Vec<u64> {
        truncate_events(events)
            .iter()
            .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
            .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
            .flatten()
            .map(|l| l.get("id").and_then(|i| i.as_u64()).unwrap_or(0))
            .collect()
    }

    #[test]
    fn archive_folds_the_marked_range_and_keeps_post_mark_rows() {
        // 行 0..3；在 2 处留档（from=4）→ 行 2、3 折叠；之后新写 4、5 仍在活动视图里。
        let events = vec![
            json!({"type":"transcript","lines":[{"id":0},{"id":1},{"id":2},{"id":3}]}),
            json!({"type":"rewind","mark":1,"mode":"archive","before":2,"from":4}),
            json!({"type":"transcript","lines":[{"id":4},{"id":5}]}),
        ];
        assert_eq!(ids(&events), vec![0, 1, 4, 5]);
        assert_eq!(next_line_id(&events), 6, "新行从最大 id + 1 续号");
        assert!(folded(&rewind_marks(&events), 3));
        assert!(!folded(&rewind_marks(&events), 4));
    }

    #[test]
    fn old_keep_marker_is_read_as_delete_and_cuts_positionally() {
        let events = vec![
            json!({"type":"transcript","lines":[{"id":0},{"id":1},{"id":2}]}),
            json!({"type":"rewind","keep":1}),
        ];
        assert_eq!(ids(&events), vec![0], "旧 keep 标记按位置截断它之前的内容");
        let m = &rewind_marks(&events)[0];
        assert_eq!(m.mode, RewindMode::Delete);
        assert_eq!(m.before, 1);
        assert_eq!(m.from, 1);
    }
}
