//! 核心端口：依赖倒置的边界。core 定义，adapters 实现，main 注入。
//! **已随能力/内核搬出**：prompt、registry、llm、workspace、tools（SysIo / ToolRunner / FenceHost）、
//! kernel（Log、HostProbe）。本文件只剩尚未搬出的部分（session）。

use crate::core::history::{HistoryView, SessionMeta};

/// 会话历史端口：一个会话一个目录（meta + 事件流水）。
/// 流水只追加；回档将来以 rewind 记录追加，不物理删行（会话状态 = 回放截断）。
pub trait HistoryStore {
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份唯一真相在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}
