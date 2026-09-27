//! 会话能力的**用例与端口持有者**：历史落盘端口只在这里（R12）。
//!
//! 别的能力要造会话 / 追流水 / 读元信息 / 删会话，走 `api::History`；
//! 呈现层的列表 / 打开 / 删除走 `api::HistoryOps`（由队列代理实现）。
//! 装配（new 出适配器）在组合根；这里只收注入的端口。

use crate::capabilities::session::api::{History, HistoryView, SessionMeta};
use crate::capabilities::session::ports::HistoryStore;
use std::sync::Arc;

/// 会话能力：持历史落盘端口，按用例答话。
pub struct SessionService {
    store: Arc<dyn HistoryStore + Send + Sync>,
}

impl SessionService {
    /// 组合根专用。
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

    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.store.list()
    }

    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.store.load(name)
    }

    fn delete(&self, name: &str) -> Result<bool, String> {
        self.store.delete(name)
    }
}
