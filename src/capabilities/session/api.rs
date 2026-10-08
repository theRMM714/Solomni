//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::session::domain::decisions::{
    issued_in, AnswerSlot, Answered, DecisionDesk, DecisionDoor, GateTicket, SlotWake,
};
pub use crate::capabilities::session::domain::events::{
    idle, interrupted_note, last_compaction, qualify, stopped_note, working, CheckView,
    DecisionCard, DecisionOption, DecisionQueue, DecisionWaiter, LineView, Live, Pending,
    SessionEvent, ToolCallView, OPT_ASK_REPLY, OPT_BEGIN, OPT_BEGIN_ALLOW, OPT_NODE_REWORK,
    OPT_NODE_SAY, OPT_PLAN_SAY, OPT_PLAN_START, OPT_SLATE_CANCEL, OPT_SLATE_CONFIRM,
    OPT_TOOL_ALLOW, OPT_TOOL_DENY, OPT_TOOL_FULL,
};
pub use crate::capabilities::session::domain::history::{
    AgentMeta, Delegation, HistoryView, RunState, SessionMeta,
};
pub use crate::capabilities::session::domain::rewind::{
    align_keep, cut_before_line, find_line_id, last_line_within, max_reply, next_line_id,
    rewind_marks, truncate_events, turn_of_line, RewindMark, RewindMode,
};
pub use crate::capabilities::session::domain::session::{
    keep_whole_replies, stream_piece, summary_message, unique_work_name, AgentSession,
    SessionParams, TurnRun,
};
pub use crate::capabilities::session::domain::tools::{tool_table, MemberTools};

// 核心操作回路 + 原生回灌消息：`service.rs` 实现（它驱动 IO，不是纯派生）。
pub use crate::capabilities::session::service::{core_operation, reply_msgs};

/// 落盘会话的**队列面**：呈现层经 conductor 的队列代理调它（列表 / 打开 / 删除）。
///
/// "在世会话 × 历史的并集"（`SessionView`）**不在本面里**：那要同时认识会话中心与历史，
/// 归会话中心（`conductor::api::SessionOps::session_views`）。
pub trait HistoryOps: Send + Sync {
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}

/// 会话的**直连面**（`service.rs` 实现）：别的能力要造会话、追流水、读元信息、删会话，走这里；
/// 出站端口 `HistoryStore`（目录布局与 append-only 文件格式）**只由它持有**（R12）。
///
/// 它与 `HistoryOps` 的分工是**接收者不同**，不是重复：`HistoryOps` 由队列代理（`ConductorHandle`）实现、
/// 面向呈现层；`History` 由能力自己实现、面向别的能力。这里的四个写操作（create / save_meta /
/// append / delete）**不开放给呈现层**——呈现层要写就经协调业务的用例。
///
/// 这一面刻意与端口**一一对应**：会话落盘没有别的不变式可编排（追加原子性、元信息唯一真相
/// 已在 `domain/history.rs` 与 store 契约里），它的价值是**唯一持有者**（R12），不是新增逻辑。
pub trait History: Send + Sync {
    /// 建一个会话目录（meta + 空流水）。
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份的**唯一真相**在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 追加若干事件（留档只走这条；删除 / 恢复走 `replace`）。
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    /// 整体重写流水（删除 / 恢复会真的截断）。
    fn replace(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    /// 列出全部落盘会话（列表页按它渲染）。
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    /// **只读元信息**（不回放流水）：运行态（暂停 / 关闭）判定走它，不为此读整份转录。
    fn meta(&self, name: &str) -> Result<SessionMeta, String>;
    /// 打开一个会话：元信息 + 事件流水（调用方按它回放状态）。
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    /// 删除一个会话目录；false = 本来就不存在。
    fn delete(&self, name: &str) -> Result<bool, String>;
}
