//! 目的：事件发布台：核心是唯一生产者，任何数量的消费者按 `since` 增量取。
//! 管：入箱与裁剪、回档重置水印、按序号取批、历史合流窗口的实时尾巴。
//! 不管：事件形状（归 `session`）、谁能写（写面只在 `turn::api::Events`）。
//! 联动：组合根造一份，写面交 `runtime`，读面交呈现层（`turn::api::EventFeed`）。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::capabilities::conversation::session::api::SessionEvent;
use crate::capabilities::turn::api::{EventFeed, EventLine, Events};

/// 目的：事件上限与保留量：本地单机足够，防无界增长。
const BUS_MAX: usize = 10_000;
const BUS_KEEP: usize = 5_000;

/// 目的：事件发布台：核心是唯一生产者（`push` 只由本能力调用）；
///   任何数量的消费者按 `since` 增量取——多端并用、断线重连都不用额外机制。
pub(crate) struct EventBus {
    inner: Mutex<BusInner>,
}

#[derive(Default)]
struct BusInner {
    seq: u64,
    lines: Vec<EventLine>,
    /// 目的：每个会话最近一次回档的重置点（事件台序号）：该序号及之前的事实已被盘上转录取代，
    /// 历史合流不得再把它当实时尾巴；客户端自己的 since 游标不受影响（它们仍需看到
    /// 重置事件本身来清空视图，见 push_rewound）。
    rewound: BTreeMap<String, u64>,
}

impl EventBus {
    /// 目的：造一个事件台：组合根装配一次，写面给运行时与核心，读面给呈现层。
    pub(crate) fn shared() -> Arc<dyn Events> {
        Arc::new(EventBus {
            inner: Mutex::new(BusInner::default()),
        })
    }

    /// 目的：入箱一批事实并裁剪，返回它的全局序号（调用方已持锁）。
    fn append_locked(g: &mut BusInner, sid: &str, events: &[SessionEvent]) -> u64 {
        g.seq += 1;
        let seq = g.seq;
        g.lines.push(EventLine {
            seq,
            sid: sid.to_string(),
            events: events.to_vec(),
        });
        if g.lines.len() > BUS_MAX {
            let drop = g.lines.len() - BUS_KEEP;
            g.lines.drain(..drop);
        }
        seq
    }

    /// 目的：历史合流窗口：只取该会话**最近一次回档之后**的事件批 + 当前头部（同一把锁内）。
    /// 回档水印之前的批次已被盘上转录取代，不能再拼回历史视图；客户端的长轮询仍用
    /// `snapshot`（它需要看到重置事件本身来清空视图）。
    fn snapshot_after_rewind(&self, sid: &str) -> (Vec<EventLine>, u64) {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let cut = g.rewound.get(sid).copied().unwrap_or(0);
        let lines = g
            .lines
            .iter()
            .filter(|l| l.sid == sid && l.seq > cut)
            .cloned()
            .collect();
        (lines, g.seq)
    }
}

impl EventFeed for EventBus {
    fn head(&self) -> u64 {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.seq
    }

    fn snapshot(&self, sid: Option<&str>, since: u64) -> (Vec<EventLine>, u64, u64) {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let lines = g
            .lines
            .iter()
            .filter(|l| l.seq > since && sid.is_none_or(|x| l.sid == x))
            .cloned()
            .collect();
        let oldest = g.lines.first().map(|l| l.seq).unwrap_or(g.seq + 1);
        (lines, g.seq, oldest)
    }

    fn tail_excluding(
        &self,
        sid: &str,
        transcript: &[serde_json::Value],
    ) -> (Vec<serde_json::Value>, u64) {
        let mut left: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for ev in transcript {
            if let Ok(k) = serde_json::to_string(ev) {
                *left.entry(k).or_insert(0) += 1;
            }
        }
        let (lines, head) = self.snapshot_after_rewind(sid);
        let mut on_bus: Vec<serde_json::Value> = lines
            .into_iter()
            .flat_map(|l| l.events.into_iter().map(|e| e.to_json()))
            .collect();
        let mut start = 0usize;
        for (i, v) in on_bus.iter().enumerate() {
            if let Ok(k) = serde_json::to_string(v)
                && let Some(n) = left.get_mut(&k)
                && *n > 0
            {
                *n -= 1;
                start = i + 1;
            }
        }
        let mut tail = Vec::new();
        for v in on_bus.drain(..).skip(start) {
            if let Ok(k) = serde_json::to_string(&v)
                && let Some(n) = left.get_mut(&k)
                && *n > 0
            {
                *n -= 1;
                continue;
            }
            tail.push(v);
        }
        (tail, head)
    }
}

impl Events for EventBus {
    fn push(&self, sid: &str, events: &[SessionEvent]) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::append_locked(&mut g, sid, events)
    }

    fn push_rewound(&self, sid: &str, events: Vec<serde_json::Value>) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ev = SessionEvent::Rewound { events };
        let seq = Self::append_locked(&mut g, sid, std::slice::from_ref(&ev));
        g.rewound.insert(sid.to_string(), seq);
        seq
    }

    #[cfg(test)]
    fn seed_for_test(&self, sid: &str) -> u64 {
        Events::push(self, sid, &[])
    }
}
