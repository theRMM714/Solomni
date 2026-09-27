//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::session::domain::events::{
    idle, interrupted_note, stopped_note, working, CheckView, LineView, Live, Pending,
    SessionEvent, ToolCallView,
};
pub use crate::capabilities::session::domain::history::{AgentMeta, HistoryView, SessionMeta};
pub use crate::capabilities::session::domain::rewind::{
    find_line_id, last_line_within, truncate_events, turn_of_line,
};
pub use crate::capabilities::session::domain::session::{
    keep_whole_replies, stream_piece, unique_work_name, AgentSession, MemberTools, ModuleTools,
    SessionParams, TurnRun,
};

/// 落盘会话的**队列面**：呈现层经 core 的队列代理调它（列表 / 打开 / 删除）。
///
/// "在世会话 × 历史的并集"（`SessionView`）**不在本面里**：那要同时认识会话中心与历史，
/// 归会话中心（`core::api::SessionOps::session_views`）。
pub trait HistoryOps: Send + Sync {
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}
