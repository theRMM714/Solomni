//! 目的：协作讨论的行外送与待裁决关的派生——把转录行变成带会话内稳定 id 的事件，把状态推成整队待答的关。
//! 管：`slate_item` / `derive_gates` / `emit_new_lines` / `push_delta` / `review_event` 五个自由函数。
//! 不管：讨论推进（`pump.rs`）、状态机步骤（`discussion.rs`）、裁决队的排队与校验（`session` 的 `Gates`）。
//! 联动：`pump` / `collab` / `slate` / `turn_io` 取同一份；会话内行 id 只在这里分配。

use crate::capabilities::collab::domain::collab_state::CollabState;
use crate::capabilities::collab::service::discussion::Discussion;
use crate::capabilities::collab::service::synthesis::Execution;
use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::session::api::{CheckView, LineView, Pending, SessionEvent};
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
    SessionEvent::Review {
        items,
        raw: exec.checklist_raw.clone(),
    }
}
