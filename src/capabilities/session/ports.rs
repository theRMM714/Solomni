//! 会话能力的**出站端口**：历史落盘（机制在适配层）。

use crate::capabilities::session::domain::history::{HistoryView, SessionMeta};

/// 会话历史端口：一个会话一个目录（meta + 事件流水）。
/// 流水只追加；回档以 rewind 记录追加，不物理删行（会话状态 = 回放截断）。
pub trait HistoryStore {
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份唯一真相在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    /// **只读元信息**（不回放流水）：派发前的运行态判定等"只看身份与运行态"的场合用它，
    /// 免得为查一个字段把整份转录读一遍。
    fn meta(&self, name: &str) -> Result<SessionMeta, String>;
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}
