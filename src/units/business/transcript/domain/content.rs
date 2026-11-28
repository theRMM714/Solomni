//! 目的：会话内容的条目与代数——只追加的条目流，读写扩展各取所需。
//! 管：条目形状（一条转录行 / 一条机制事实）、线格式与条目互转、行号与回复号的推进、
//!   折叠（读时）与截断（写时，只有版本控制的删除 / 恢复用）、最后一次压缩与上下文用量。
//! 不管：条目何时产生（各机制的写者）、落在哪个文件（在会话单元的 detail）、折叠区间代表什么业务（调用方解释）。
//! 联动：conversation::session 经 History 端口读写它；lifecycle 的回档、turn 的重建、work 的视图都读它。

use crate::units::business::transcript::domain::events::{ContextUsage, LineView};

/// 目的：一条条目：转录行、机制事实，或认不出的线格式（原样保留，读写不丢数据）。
#[derive(Debug, Clone)]
pub enum Entry {
    /// 目的：一条转录行（id 是会话内稳定序号）。
    /// 约束：装箱——行比别两种条目大得多，不装箱会让每条事实都按行的大小占位。
    Line(Box<LineView>),
    /// 目的：一条机制事实：线上 type 铺在顶层的那类事件（用量、留档标记、流程推送、裁决…）。
    Fact {
        /// 目的：线上的 type。
        kind: String,
        /// 目的：整条事件原样（type 也在里面），写回时不再拼一次。
        body: serde_json::Value,
    },
    /// 目的：认不出的线格式：原样保留。
    Raw(serde_json::Value),
}

/// 目的：一个会话的内容：按落盘顺序排好的条目流，只追加。
/// 约束：本体不解释事实的业务含义（kind 只有写者与读方认识）；折叠与截断是它唯一的两类变换。
#[derive(Debug, Clone, Default)]
pub struct Content {
    entries: Vec<Entry>,
}

impl Content {
    /// 目的：空内容。
    pub fn empty() -> Content {
        Content {
            entries: Vec::new(),
        }
    }

    /// 目的：把线格式（transcript 事件里的行 + 其余事实）读成条目。
    /// 参数：events = 落盘流水里的 JSON 事件。
    /// 约束：认不出的行与事件留在 Raw 里，下一次写回原样出现。
    pub fn of_events(events: &[serde_json::Value]) -> Content {
        let mut entries = Vec::new();
        for ev in events {
            let kind = ev.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if kind == "transcript" {
                match ev.get("lines").and_then(|l| l.as_array()) {
                    Some(lines) => {
                        for line in lines {
                            match serde_json::from_value::<LineView>(line.clone()) {
                                Ok(view) => entries.push(Entry::Line(Box::new(view))),
                                Err(_) => entries.push(Entry::Raw(line.clone())),
                            }
                        }
                    }
                    None => entries.push(Entry::Raw(ev.clone())),
                }
                continue;
            }
            entries.push(Entry::Fact {
                kind: kind.to_string(),
                body: ev.clone(),
            });
        }
        Content { entries }
    }

    /// 目的：写回线格式：相邻的行并成一条 transcript 事件，事实与认不出的条目各自成条。
    pub fn to_events(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let mut batch: Vec<serde_json::Value> = Vec::new();
        fn flush(batch: &mut Vec<serde_json::Value>, out: &mut Vec<serde_json::Value>) {
            if batch.is_empty() {
                return;
            }
            let lines = std::mem::take(batch);
            out.push(serde_json::json!({ "type": "transcript", "lines": lines }));
        }
        for entry in &self.entries {
            match entry {
                Entry::Line(l) => match serde_json::to_value(l) {
                    Ok(v) => batch.push(v),
                    Err(_) => continue,
                },
                Entry::Fact { body, .. } => {
                    flush(&mut batch, &mut out);
                    out.push(body.clone());
                }
                Entry::Raw(v) => {
                    flush(&mut batch, &mut out);
                    out.push(v.clone());
                }
            }
        }
        flush(&mut batch, &mut out);
        out
    }

    /// 目的：全部转录行（按落盘顺序）。
    pub fn lines(&self) -> impl Iterator<Item = &LineView> {
        self.entries.iter().filter_map(|e| match e {
            Entry::Line(l) => Some(l.as_ref()),
            _ => None,
        })
    }

    /// 目的：按稳定 id 找一行。
    pub fn line(&self, id: u64) -> Option<&LineView> {
        self.lines().find(|l| l.id == id)
    }

    /// 目的：追加一条行（写扩展共用的基础写）。
    pub fn push_line(&mut self, line: LineView) {
        self.entries.push(Entry::Line(Box::new(line)));
    }

    /// 目的：追加一条事实（kind 由写扩展自己定，本体不解释）。
    /// 参数：kind = 线上 type；body = 铺平后的载荷（不含 type，这里补上）。
    pub fn push_fact(&mut self, kind: &str, mut body: serde_json::Value) {
        if let serde_json::Value::Object(m) = &mut body {
            m.insert(
                "type".to_string(),
                serde_json::Value::String(kind.to_string()),
            );
        }
        self.entries.push(Entry::Fact {
            kind: kind.to_string(),
            body,
        });
    }

    /// 目的：某个 kind 的全部事实（按落盘顺序）。
    pub fn facts(&self, kind: &str) -> Vec<&serde_json::Value> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Fact { kind: k, body } if k == kind => Some(body),
                _ => None,
            })
            .collect()
    }

    /// 目的：下一条新行的 id（用过的最大 id + 1；不重用折叠行的号）。
    pub fn next_id(&self) -> u64 {
        self.lines().map(|l| l.id + 1).max().unwrap_or(0)
    }

    /// 目的：用过的最大回复号（重建时据此续号）。
    pub fn max_reply(&self) -> u64 {
        self.lines()
            .flat_map(|l| [l.reply, l.tool.as_ref().map(|t| t.reply).unwrap_or(0)])
            .max()
            .unwrap_or(0)
    }

    /// 目的：某一行属于哪个回合（0 = 不属任何回合）。
    pub fn turn_of(&self, id: u64) -> u64 {
        self.line(id).map(|l| l.turn).unwrap_or(0)
    }

    /// 目的：保留到某个回合为止时，该用的保留 id：第一行超出保留点的行的 id。
    /// 约束：回合 0 的行跟着它前面那一回合走（用户插话、需求等因此不会被误删）。
    pub fn keep_at_turn(&self, turn: u64) -> u64 {
        let mut cut = 0u64;
        for l in self.lines() {
            if l.turn > turn {
                return l.id;
            }
            cut = l.id + 1;
        }
        cut
    }

    /// 目的：把保留点对齐到回复边界（落在某次回复内部时退到该回复第一行之前）。
    /// 返回：对齐后的保留 id；没有这样的行 = u64::MAX（不截）。
    pub fn align_keep(&self, keep: u64) -> u64 {
        let rows: Vec<&LineView> = self.lines().collect();
        let idx = rows.iter().position(|l| l.id >= keep).unwrap_or(rows.len());
        let replies: Vec<u64> = rows.iter().map(|l| line_reply_of(l)).collect();
        let aligned = keep_whole_replies(&replies, idx);
        rows.get(aligned).map(|l| l.id).unwrap_or(u64::MAX)
    }

    /// 目的：读时变换：藏掉落在区间里的行（区间由调用方解释，如留档折叠）。
    /// 参数：ranges = 半开区间 [before, from) 的列表。
    /// 约束：带 line 的事实随那一行一起藏；不带的事实（如留档标记本身）留着。
    pub fn hide(&self, ranges: &[(u64, u64)]) -> Content {
        let hidden = |id: u64| {
            ranges
                .iter()
                .any(|(before, from)| *before <= id && id < *from)
        };
        let entries = self
            .entries
            .iter()
            .filter(|e| match e {
                Entry::Line(l) => !hidden(l.id),
                Entry::Fact { body, .. } => body
                    .get("line")
                    .and_then(|l| l.as_u64())
                    .map(|id| !hidden(id))
                    .unwrap_or(true),
                Entry::Raw(_) => true,
            })
            .cloned()
            .collect();
        Content { entries }
    }

    /// 目的：写时变换：只保留 id < keep 的条目（回档删除 / 恢复才用，属版本控制）。
    /// 参数：keep = 保留 id。
    /// 约束：先对齐回复边界；第一条被截的行之后的事实一并丢弃（事件流是时序的）。
    pub fn cut_before(&self, keep: u64) -> Content {
        let keep = self.align_keep(keep);
        let mut entries = Vec::new();
        for e in &self.entries {
            match e {
                Entry::Line(l) => {
                    if l.id >= keep {
                        break;
                    }
                    entries.push(e.clone());
                }
                Entry::Fact { body, .. } => {
                    let line = body.get("line").and_then(|v| v.as_u64());
                    if line.map(|id| id >= keep).unwrap_or(false) {
                        continue;
                    }
                    entries.push(e.clone());
                }
                Entry::Raw(v) => entries.push(Entry::Raw(v.clone())),
            }
        }
        Content { entries }
    }

    /// 目的：恢复用：只保留某条留档标记之前的条目（标记本身在它之后，一并去掉）。
    /// 返回：找到那条标记 = 截到它之前；没有 = None。
    pub fn up_to_mark(&self, mark: u64) -> Option<Content> {
        let idx = self.entries.iter().position(|e| match e {
            Entry::Fact { kind, body } => {
                kind == "rewind" && body.get("mark").and_then(|m| m.as_u64()) == Some(mark)
            }
            _ => false,
        })?;
        Some(Content {
            entries: self.entries[..idx].to_vec(),
        })
    }

    /// 目的：流水里的全部留档标记（按出现顺序）。
    pub fn marks(&self) -> Vec<ArchiveMark> {
        self.facts("rewind")
            .into_iter()
            .filter_map(ArchiveMark::from_value)
            .collect()
    }

    /// 目的：读时变换：被「改需求」取代的旧行不进内容（历史仍在，审计看得到）。
    /// 约束：取代关系由写扩展追加的 `supersede` 事实声明（targets = 旧行的 id）。
    pub fn superseded(&self) -> Content {
        let mut ranges = Vec::new();
        for ev in self.facts("supersede") {
            let Some(targets) = ev.get("targets").and_then(|v| v.as_array()) else {
                continue;
            };
            for t in targets {
                if let Some(id) = t.as_u64() {
                    // 单行区间：hide 用半开区间 [id, id+1)。
                    ranges.push((id, id + 1));
                }
            }
        }
        if ranges.is_empty() {
            return self.clone();
        }
        self.hide(&ranges)
    }

    /// 目的：留档折叠后的活动视图：折叠区间里的行不进内容（文件里仍在，恢复后回来）。
    pub fn folded_away(&self) -> Content {
        let ranges: Vec<(u64, u64)> = self.marks().iter().map(|m| (m.before, m.from)).collect();
        self.hide(&ranges)
    }

    /// 目的：留档标记给用户看的标签——折叠区间里最后一条内容行，截到 40 个字符。
    /// 约束：纯派生；工具行 / 提示行 / 压缩行不算；没有时退回「第 N 行」。
    pub fn archive_label(&self, before: u64, from: u64) -> String {
        let mut found = String::new();
        for l in self.lines() {
            if l.id < before || l.id >= from {
                continue;
            }
            if l.kind == "compacted" || l.kind == "notice" || l.tool.is_some() {
                continue;
            }
            let text = l.line.trim();
            if !text.is_empty() {
                found = text.to_string();
            }
        }
        if found.is_empty() {
            return format!("第 {} 行", before);
        }
        found.chars().take(40).collect()
    }

    /// 目的：最后一次压缩（up_to、摘要、行 id）。
    /// 约束：回档到压缩行之前时它已随转录被截掉，所以「没有它」就是「回到压缩前」。
    pub fn last_compaction(&self) -> Option<(u64, String, u64)> {
        let mut found = None;
        for l in self.lines() {
            if l.kind != "compacted" {
                continue;
            }
            let up_to = l
                .payload
                .as_ref()
                .and_then(|p| p.get("up_to"))
                .and_then(|u| u.as_u64())
                .unwrap_or(0);
            if up_to > 0 && !l.line.is_empty() {
                found = Some((up_to, l.line.clone(), l.id));
            }
        }
        found
    }

    /// 目的：活动视图里最后一次模型调用报的上下文用量（没有 = None）。
    pub fn last_usage(&self) -> Option<ContextUsage> {
        let folded: Vec<(u64, u64)> = self.marks().iter().map(|m| (m.before, m.from)).collect();
        self.facts("usage").into_iter().rev().find_map(|ev| {
            // 折叠区间里的用量跟着那一行走：不算当前视图。
            let line = ev.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
            if folded
                .iter()
                .any(|(before, from)| *before <= line && line < *from)
            {
                return None;
            }
            Some(ContextUsage {
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
}

/// 目的：把「保留前 keep 行」对齐到回复边界：keep 落在某次回复内部时退到该回复第一行之前。
/// 约束：转录截断与内存历史截断必须用同一个函数，否则前端事件流与模型上下文会不一致。
pub fn keep_whole_replies(line_reply: &[u64], keep: usize) -> usize {
    let mut keep = keep.min(line_reply.len());
    if keep > 0 && keep < line_reply.len() {
        let losing = line_reply[keep];
        while keep > 0 && line_reply[keep - 1] == losing {
            keep -= 1;
        }
    }
    keep
}

/// 目的：一行属于哪次回复：工具行的号在调用视图里，其余行在 LineView 上。
/// 约束：号 = 0 视为「没写」（回复号从 1 起），此时用该行自己的 id，绝不把相邻行误并成一组。
pub fn line_reply_of(l: &LineView) -> u64 {
    let stored = match &l.tool {
        Some(t) => t.reply,
        None => l.reply,
    };
    if stored != 0 { stored } else { l.id }
}

/// 目的：流水里一条留档标记（只追加；恢复 = 在它之前截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMark {
    /// 目的：标记的单调 id（呈现与恢复按它定位）。
    pub mark: u64,
    /// 目的：保留 id < before 的行。
    pub before: u64,
    /// 目的：标记之后新行从此起的 id（留档用；删除模式等于 before）。
    pub from: u64,
    /// 目的：留档前的主副本头（None = 空共享区）。
    pub shared: Option<u64>,
    /// 目的：留档前各 agent 沙箱的物化点（档案 ref 的同一份）。
    pub heads: std::collections::BTreeMap<String, Option<u64>>,
    /// 目的：给用户看的一句话（折叠区间里最后一条内容行）。
    pub label: String,
}

impl ArchiveMark {
    /// 目的：从一条事实解析；不是留档记录 = None。
    pub fn from_value(ev: &serde_json::Value) -> Option<ArchiveMark> {
        if ev.get("type").and_then(|t| t.as_str()) != Some("rewind") {
            return None;
        }
        let before = ev.get("before").and_then(|v| v.as_u64())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_round_trip_line_by_line() {
        let events = vec![
            json!({"type":"transcript","lines":[{"id":0,"kind":"msg","line":"甲"},{"id":1,"kind":"msg","line":"乙"}]}),
            json!({"type":"usage","line":0,"prompt_tokens":3}),
            json!({"type":"transcript","lines":[{"id":2,"kind":"msg","line":"丙"}]}),
        ];
        let c = Content::of_events(&events);
        assert_eq!(c.lines().count(), 3);
        // 线格式写回后再读一遍必须稳定（相邻行并成一条、事实各自成条）。
        let back = c.to_events();
        assert_eq!(Content::of_events(&back).to_events(), back);
        assert_eq!(back.len(), 3, "两段行 + 一条用量事实");
    }

    #[test]
    fn a_line_id_is_never_reused_by_folding() {
        let events = vec![
            json!({"type":"transcript","lines":[{"id":0},{"id":1},{"id":2},{"id":3}]}),
            json!({"type":"rewind","mark":1,"mode":"archive","before":2,"from":4}),
            json!({"type":"transcript","lines":[{"id":4},{"id":5}]}),
        ];
        let c = Content::of_events(&events);
        let ids: Vec<u64> = c.folded_away().lines().map(|l| l.id).collect();
        assert_eq!(ids, vec![0, 1, 4, 5]);
        assert_eq!(c.next_id(), 6, "新行从最大 id + 1 续号");
        assert!(c.marks()[0].before <= 3 && 3 < c.marks()[0].from);
    }

    #[test]
    fn cutting_keeps_only_the_rows_before_the_point() {
        let events = vec![
            json!({"type":"transcript","lines":[{"id":0,"line":"甲"},{"id":1,"line":"乙"}]}),
            json!({"type":"usage","line":1,"prompt_tokens":3}),
            json!({"type":"transcript","lines":[{"id":2,"line":"丙"}]}),
        ];
        let c = Content::of_events(&events).cut_before(1);
        assert_eq!(c.lines().count(), 1);
        assert_eq!(c.to_events().len(), 1, "截点之后的事件一并丢弃");
    }

    #[test]
    fn a_fact_without_a_line_survives_folding() {
        let events = vec![
            json!({"type":"transcript","lines":[{"id":0},{"id":1},{"id":2}]}),
            json!({"type":"rewind","mark":1,"mode":"archive","before":0,"from":2}),
        ];
        let view = Content::of_events(&events).folded_away();
        assert_eq!(view.lines().map(|l| l.id).collect::<Vec<_>>(), vec![2]);
        assert_eq!(view.facts("rewind").len(), 1);
    }
}
