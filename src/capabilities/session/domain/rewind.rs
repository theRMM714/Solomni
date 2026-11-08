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

/// 流水里一条留档标记（只追加；恢复 = 在它之前截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMark {
    /// 标记的单调 id（呈现与恢复按它定位）。
    pub mark: u64,
    /// 保留 id < before 的行。
    pub before: u64,
    /// 标记之后新行从此起的 id（留档用；删除模式等于 before）。
    pub from: u64,
    /// 留档前的主副本头（None = 空共享区）。
    pub shared: Option<u64>,
    /// 留档前各 agent 沙箱的物化点（档案 ref 的同一份）。
    pub heads: std::collections::BTreeMap<String, Option<u64>>,
    /// 给用户看的一句话（折叠区间里最后一条内容行）。
    pub label: String,
}

impl ArchiveMark {
    /// 从一条流水事件解析；不是留档记录 = None。
    /// 当前格式只有留档标记：`before`（保留 id < before）与 `mode: archive`。
    pub fn from_value(ev: &serde_json::Value) -> Option<ArchiveMark> {
        if ev.get("type").and_then(|t| t.as_str()) != Some("rewind") {
            return None;
        }
        let before = ev.get("before").and_then(|v| v.as_u64())?;
        // 只有留档标记带 mode=archive；删除与恢复都不写标记。
        if ev.get("mode").and_then(|m| m.as_str()) != Some("archive") {
            return None;
        }
        let heads = ev
            .get("heads")
            .and_then(|h| h.as_object())
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_u64())).collect())
            .unwrap_or_default();
        Some(ArchiveMark {
            mark: ev.get("mark").and_then(|v| v.as_u64()).unwrap_or(0),
            before,
            from: ev.get("from").and_then(|v| v.as_u64()).unwrap_or(before),
            shared: ev.get("shared").and_then(|v| v.as_u64()),
            heads,
            label: ev
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// 流水里全部留档标记（按出现顺序）。恢复 = 在该标记处截断文件，所以更晚的标记随之一并消失。
pub fn rewind_marks(events: &[serde_json::Value]) -> Vec<ArchiveMark> {
    events.iter().filter_map(ArchiveMark::from_value).collect()
}

/// 目的：留档标记给用户看的标签——折叠区间里最后一条内容行，截到 40 个字符。
/// 约束：纯派生；没有内容行时退回"第 N 行"，不写时间也不写随机量。
pub fn archive_label(events: &[serde_json::Value], before: u64, from: u64) -> String {
    let mut found = String::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            let Some(id) = l.get("id").and_then(|i| i.as_u64()) else {
                continue;
            };
            if id < before || id >= from {
                continue;
            }
            let kind = l.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if matches!(kind, "compacted" | "notice") || l.get("tool").is_some() {
                continue;
            }
            let text = l.get("line").and_then(|x| x.as_str()).unwrap_or("").trim();
            if !text.is_empty() {
                found = text.to_string();
            }
        }
    }
    if found.is_empty() {
        return format!("第 {} 行", before);
    }
    found.chars().take(40).collect()
}

/// 一行是否被某个**留档**标记折叠（落在它的 [before, from) 内）。
pub fn folded(marks: &[ArchiveMark], id: u64) -> bool {
    marks.iter().any(|m| m.before <= id && id < m.from)
}

/// 应用回档标记后的**活动视图**：
/// - 留档折叠的行不进内容（模型与界面都看不到；文件里仍在，恢复后回来）；
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
            // 用量事件也跟着它那次调用的首行走：行被折叠了，这条用量就不再代表当前发送视图。
            Some("usage") => {
                let line = ev.get("line").and_then(|i| i.as_u64()).unwrap_or(0);
                if !folded(&marks, line) {
                    content.push(ev.clone());
                }
            }
            Some("rewind") => content.push(ev.clone()),
            _ => content.push(ev.clone()),
        }
    }
    content
}

/// 目的：活动视图里最后一次模型调用报的上下文用量（没有 = None）。
/// 约束：折叠区间里的用量不算；回档到它之前时状态栏因此不会继续显示旧数字。
pub fn last_usage(
    events: &[serde_json::Value],
) -> Option<crate::capabilities::session::api::ContextUsage> {
    let marks = rewind_marks(events);
    events.iter().rev().find_map(|ev| {
        if ev.get("type").and_then(|t| t.as_str()) != Some("usage") {
            return None;
        }
        let line = ev.get("line").and_then(|i| i.as_u64()).unwrap_or(0);
        if folded(&marks, line) {
            return None;
        }
        Some(crate::capabilities::session::api::ContextUsage {
            prompt_tokens: ev
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            completion_tokens: ev
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            total_tokens: ev.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            window: ev.get("window").and_then(|v| v.as_u64()).unwrap_or(0),
            exact: ev.get("exact").and_then(|v| v.as_bool()).unwrap_or(false),
            cache_read: ev.get("cache_read").and_then(|v| v.as_u64()),
            cache_creation: ev.get("cache_creation").and_then(|v| v.as_u64()),
        })
    })
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
        } else if ev.get("type").and_then(|t| t.as_str()) == Some("usage") {
            // 用量属于它那次调用的首行；保留点之前的才留下。
            let line = ev.get("line").and_then(|i| i.as_u64()).unwrap_or(0);
            if line < keep {
                out.push(ev.clone());
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

    /// 目的：标签取折叠区间里最后一条内容行——工具行 / 提示行 / 压缩提示不算，超长截到 40 字。
    #[test]
    fn archive_label_takes_the_last_content_line_in_the_folded_range() {
        let events = vec![json!({"type":"transcript","lines":[
            {"id":0,"kind":"msg","line":"第一句"},
            {"id":1,"kind":"tool","line":"[工具]","tool":{}},
            {"id":2,"kind":"msg","line":"最后一句"},
            {"id":3,"kind":"notice","line":"[提示] x"},
            {"id":4,"kind":"msg","line":"范围外"}
        ]})];
        assert_eq!(archive_label(&events, 0, 4), "最后一句");
        assert_eq!(archive_label(&events, 4, 9), "范围外");
        assert_eq!(
            archive_label(&events, 0, 0),
            "第 0 行",
            "没有内容行时退回行号"
        );
        let long = "字".repeat(60);
        let events = vec![json!({"type":"transcript","lines":[{"id":0,"kind":"msg","line":long}]})];
        assert_eq!(archive_label(&events, 0, 1).chars().count(), 40);
    }

    /// 目的：留档标记解析出共享区头、每个 agent 的头与标签；非留档记录一律不算标记。
    #[test]
    fn archive_mark_reads_shared_heads_and_label_only_for_archive_rows() {
        let ev = json!({"type":"rewind","mark":3,"mode":"archive","before":5,"from":8,
            "shared":7,"heads":{"a":7,"b":null},"label":"改完了"});
        let m = ArchiveMark::from_value(&ev).expect("留档记录");
        assert_eq!((m.mark, m.before, m.from, m.shared), (3, 5, 8, Some(7)));
        assert_eq!(m.heads.get("a"), Some(&Some(7)));
        assert_eq!(m.heads.get("b"), Some(&None));
        assert_eq!(m.label, "改完了");
        assert!(ArchiveMark::from_value(&json!({"type":"rewind","mode":"delete"})).is_none());
        assert!(ArchiveMark::from_value(&json!({"type":"notice"})).is_none());
    }
}
