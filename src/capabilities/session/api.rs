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
