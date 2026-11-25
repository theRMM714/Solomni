//! 目的：会话能力的用例与端口持有者——历史落盘端口只在这里（R12）。
//! 管：造会话 / 追流水 / 读元信息 / 删会话的用例，会话装配（`assemble`）、核实环境（`verify`）、增量落盘（`persist_events` / `record_events` / `Persister`）。
//! 不管：文件机制（在 `detail/fs_history.rs`）、会话运行时与回档算术（在 `agentloop`）、队列代理（在 runtime）。
//! 联动：别的能力走 `api::History`；呈现层走 `api::HistoryOps`（由队列代理实现）；装配在组合根，这里只收注入的端口。

use crate::units::conversation::session::ports::HistoryStore;
use crate::units::session_model::api::persist_policy_for;
use crate::units::session_model::api::{HistoryView, SessionMeta};
use crate::units::transcript::api::SessionEvent;
use crate::units::turn_state::api::History;
use std::sync::Arc;

pub mod assemble;
pub(crate) mod tree;
pub mod verify;

/// 目的：会话能力：持历史落盘端口，按用例答话。
pub struct SessionService {
    store: Arc<dyn HistoryStore + Send + Sync>,
}

impl SessionService {
    /// 目的：组合根专用。
    pub fn new(store: Arc<dyn HistoryStore + Send + Sync>) -> SessionService {
        SessionService { store }
    }
}

impl History for SessionService {
    fn create(&self, meta: &SessionMeta) -> Result<(), String> {
        self.store.create(meta)
    }

    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        self.store.save_meta(meta)
    }

    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.store.append(name, events)
    }

    fn replace(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.store.replace(name, events)
    }

    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.store.list()
    }

    fn meta(&self, name: &str) -> Result<SessionMeta, String> {
        self.store.meta(name)
    }

    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.store.load(name)
    }

    fn delete(&self, name: &str) -> Result<bool, String> {
        self.store.delete(name)
    }
}

/// 目的：事件落盘（按该会话的策略；短暂事件不落盘）：失败如实告知，返回要外送的警告（None = 一切正常）。
/// 参数：history = 落盘端口；log = 失败记账处；sid = 会话名；events = 待落事件。
/// 约束：工作线程按"一次模型调用"的粒度增量落盘，与核心走同一段逻辑——两条路径各写一份的话，
///   "什么算定稿、什么不落盘"迟早会不一致。
pub fn persist_events(
    history: &dyn History,
    log: &dyn crate::kernel::ports::Log,
    sid: &str,
    events: &[SessionEvent],
) -> Option<String> {
    // 留不留由**这个会话的策略**说了算（会话种类 → 策略，见 persist_policy_for）。
    let policy = persist_policy_for(sid);
    let jsons: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| policy.keeps(e))
        .map(|e| e.to_json())
        .collect();
    if jsons.is_empty() {
        return None;
    }
    match history.append(sid, &jsons) {
        Ok(()) => None,
        Err(e) => {
            log.error(
                "session::history_append",
                &format!("会话 {} 落盘失败：{}", sid, e),
            );
            Some(format!("[警告] 会话记录落盘失败：{}", e))
        }
    }
}

/// 目的：事件落盘并如实告知：失败时把警告**追加成一条瞬时提示**，不静默丢历史。
pub fn record_events(
    history: &dyn History,
    log: &dyn crate::kernel::ports::Log,
    sid: &str,
    events: &mut Vec<SessionEvent>,
) {
    if events.is_empty() {
        return;
    }
    if let Some(warn) = persist_events(history, log, sid, events) {
        events.push(SessionEvent::Notice(warn));
    }
}

/// 目的：增量落盘手柄：工作线程按"一次模型调用"的粒度把定稿事件落盘。
///  为什么要它：整段生成跑完才落一次盘，会让中途刷新看不到已经产生的部分。
#[derive(Clone)]
pub struct Persister {
    history: std::sync::Arc<dyn History + Send + Sync>,
    log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    sid: String,
}

impl Persister {
    /// 目的：组合根与调用方专用：绑定一条会话的落盘端口与日志。
    pub fn new(
        history: std::sync::Arc<dyn History + Send + Sync>,
        log: std::sync::Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        sid: &str,
    ) -> Persister {
        Persister {
            history,
            log,
            sid: sid.to_string(),
        }
    }

    /// 目的：落盘这一批；返回要外送给用户的警告（None = 正常）。
    pub fn persist(&self, events: &[SessionEvent]) -> Option<String> {
        persist_events(self.history.as_ref(), self.log.as_ref(), &self.sid, events)
    }
}
