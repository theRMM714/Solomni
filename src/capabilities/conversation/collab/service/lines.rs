//! 目的：协作讨论的行外送与待裁决关的派生——把转录行变成带会话内稳定 id 的事件，把状态推成整队待答的关。
//! 管：`slate_item` / `derive_gates` / `emit_new_lines` / `push_delta` / `review_event` 五个自由函数。
//! 不管：讨论推进（`pump.rs`）、状态机步骤（`discussion.rs`）、裁决队的排队与校验（`session` 的 `Gates`）。
//! 联动：`pump` / `collab` / `slate` / `turn_io` 取同一份；会话内行 id 只在这里分配。

use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::conversation::collab::domain::collab_state::CollabState;
use crate::capabilities::conversation::collab::domain::events::{CheckView, CollabEvent};
use crate::capabilities::conversation::collab::domain::gates::Pending;
use crate::capabilities::conversation::collab::service::discussion::Discussion;
use crate::capabilities::conversation::collab::service::synthesis::Execution;
use crate::capabilities::conversation::session::api::{LineView, SessionEvent};
/// 目的：代拟行里的一项（只给人看）：复用项标出来，组装项带上模块与模型。
pub(crate) fn slate_item(a: &AgentMeta, why: &str) -> String {
    if a.transient {
        format!(
            "{}〈{}〉→ {}（{}）",
            a.name,
            a.modules.join(","),
            a.model.clone().unwrap_or_default(),
            why
        )
    } else {
        format!("{}（复用；{}）", a.name, why)
    }
}

/// 目的：从派生状态推出**当前整队**待用户裁决的关（空 = 没有待用户处理的门）：队首在最前。
///   判据两条并用：**转录里没人回答、也没作废的那几张卡**优先（按先来后到，各带卡号与这一关的载荷）；
///   一张都没有时退回**状态派生**——回档把卡连同那一段转录一起截掉、或停会话把整队作废之后，
///   状态照样说得出"还停在名单确认 / 开始讨论"，回来时续一个新卡号接着问。
pub(crate) fn derive_gates(st: &CollabState) -> Vec<(Option<String>, Pending, String)> {
    if st.ended {
        return Vec::new();
    }
    let mut out: Vec<(Option<String>, Pending, String)> = st
        .open_gates
        .iter()
        .filter_map(|(id, gate, payload)| {
            let p = Pending::from_payload(gate, payload)?;
            let advice = payload
                .get("advice")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some((Some(id.clone()), p, advice))
        })
        .collect();
    if out.is_empty() && !st.begun {
        if st.slate.is_some() && !st.slate_confirmed {
            out.push((None, Pending::ConfirmSlate, String::new()));
        } else if st.task.is_some() {
            out.push((None, Pending::ConfirmBegin, String::new()));
        }
    }
    out
}

/// 目的：逐成员外送：把刚定稿的讨论行变成带**会话内稳定 id** 的转录事件交出去。
///   为什么要 Cell/RefCell：回调在 `Discussion::step/open` 内部被调用，那时 `self` 正被可变借用，
///   碰不到 `self.next_line` 与 `sink`——所以调用前后各并回一次，行只构造一次。
pub(crate) fn emit_new_lines(
    lines: &[LineView],
    next_line: &std::cell::Cell<u64>,
    handed: &std::cell::Cell<usize>,
    sink: &mut dyn FnMut(SessionEvent),
) {
    let views: Vec<LineView> = lines
        .iter()
        .map(|l| {
            // 会话内稳定 id 由**主会话**在行的第一次见光时分配（行自己不带 id）。
            let id = next_line.get();
            next_line.set(id + 1);
            LineView {
                id,
                reply: id,
                ..l.clone()
            }
        })
        .collect();
    handed.set(handed.get() + views.len());
    if !views.is_empty() {
        sink(SessionEvent::Transcript(views));
    }
}

/// 目的：发出自上次以来的新转录行（增量），逐行分配会话内稳定 id。
pub(crate) fn push_delta(
    disc: &Discussion,
    emitted: &mut usize,
    next_line: &mut u64,
    sink: &mut dyn FnMut(SessionEvent),
) {
    if disc.transcript.len() > *emitted {
        let views: Vec<LineView> = disc.transcript[*emitted..]
            .iter()
            .map(|l| {
                // 整行照搬（工具视图 / 降级 / 回合号 / 系统标记都不丢）。
                let v = LineView {
                    id: *next_line,
                    reply: *next_line,
                    ..l.clone()
                };
                *next_line += 1;
                v
            })
            .collect();
        *emitted = disc.transcript.len();
        sink(SessionEvent::Transcript(views));
    }
}

pub(crate) fn review_event(exec: &Execution) -> SessionEvent {
    let items = exec
        .items
        .iter()
        .map(|i| CheckView {
            item: i.item.clone(),
            status: i.status.clone(),
            note: i
                .reason
                .clone()
                .or_else(|| i.evidence.clone())
                .unwrap_or_default(),
        })
        .collect();
    CollabEvent::Review {
        items,
        raw: exec.checklist_raw.clone(),
    }
    .push()
}

/// 目的：总验收清单的**人看文本**（Markdown，一条一行）：以转录行落档，刷新与回档后照样读得到。
///   约束：结论词只有"通过 / 未通过"两档，判据与 `Execution::all_pass` 同一条（只认 `pass`）；
///   事件仍保留结构化条目（流程视图与状态派生读它，不解析这段文本）。
pub(crate) fn review_text(exec: &Execution) -> String {
    if exec.items.is_empty() {
        return format!("清单解析失败，原文：\n{}", exec.checklist_raw);
    }
    let mut out = String::new();
    for it in &exec.items {
        if it.status.eq_ignore_ascii_case("pass") {
            out.push_str(&format!("- **{}** · 通过", it.item));
        } else {
            match it
                .rework
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
            {
                Some(who) => out.push_str(&format!("- **{}** · 未通过，返工 {who}", it.item)),
                None => out.push_str(&format!("- **{}** · 未通过", it.item)),
            }
        }
        let note = it
            .reason
            .as_deref()
            .or(it.evidence.as_deref())
            .map(str::trim)
            .filter(|n| !n.is_empty());
        if let Some(note) = note {
            out.push_str(&format!(" —— {note}"));
        }
        out.push('\n');
    }
    out
}
