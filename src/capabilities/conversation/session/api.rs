//! 目的：会话能力的入站面——其它能力与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：重导出会话事实（元信息与视图、回档算术、工具面、裁决队列、事件词汇、落盘策略）、会话装配（参数对象与整场入口）、核心核实环境与核心操作回路；定义 `History` / `HistoryOps` 两个面。
//! 不管：`domain` / `ports` 的内部实现；`HistoryOps` 的队列代理实现（在 `runtime`）。
//! 联动：呈现层经 runtime 的队列代理调 `HistoryOps`；别的能力经 `History` 造会话、追流水、读元信息、删会话。

/// 目的：测试直调工具批执行器；生产路径只经会话引擎，不从这里取。
#[cfg(test)]
pub(crate) use crate::capabilities::conversation::session::service::engine::tools::run_batch;

pub use crate::capabilities::conversation::session::domain::decisions::{
    Answered, Decisions, GateTicket, Gates, SlotWake, decisions, issued_in,
};
pub use crate::capabilities::conversation::session::domain::events::{
    ContextUsage, DecisionCard, DecisionEnvelope, DecisionMessage, DecisionQueue, DecisionWaiter,
    Gate, LineView, OptionView, SessionEvent, ToolCallView, idle, interrupted_note,
    last_compaction, qualify, stopped_note, tool_ask, working,
};
// 机制面：驱动只认这三个名字（推一步 / 让出一回合 / 交回后的处置）。
pub use crate::capabilities::conversation::session::domain::history::{
    Delegation, HistoryView, RunState, SessionMeta, UsageTotals, WorkspaceBinding,
};
pub use crate::capabilities::conversation::session::domain::mechanism::{
    Handback, Mechanism, MemberAsk, Settle,
};
// 会话类型表：组合根装载它，装配期用它校验角色与机制。
pub use crate::capabilities::conversation::session::domain::profile::Profiles;
// 工具面：角色面 → 实际可调的工具（档位只管网络，不再逐工具裁剪）。
#[allow(unused_imports)]
pub use crate::capabilities::conversation::session::domain::assemble::face_tools;
pub use crate::capabilities::conversation::session::domain::mode::Mode;
#[allow(unused_imports)]
pub use crate::capabilities::conversation::session::domain::persist::{
    PersistPolicy, SYSTEM_SID_SUGGEST, is_system_session, persist_policy_for,
};
pub use crate::capabilities::conversation::session::domain::rewind::{
    align_keep, archive_label, cut_before_line, last_line_within, last_usage, max_reply,
    next_line_id, rewind_marks, truncate_events, turn_of_line,
};
pub use crate::capabilities::conversation::session::domain::session::{
    AgentSession, CompactChatFactory, CompactChatSpec, CompactionPlan, FreshnessStats, ReadAt,
    SessionParams, TurnRun, delta_visible, freshness_view, keep_whole_replies, stream_piece,
    summary_message, unique_work_name,
};
pub use crate::capabilities::conversation::session::domain::tools::{
    CoreVerify, ModuleTools, ServiceOp, ShellLive, ShellSlot, ToolFace, last_shell,
    service_ops_for_modules, tool_table,
};
// 会话装配：参数对象与三个整场入口（加上只取工具面值的 `member_face`）。
// `MemberTools`（工具面的活对象）**不经这里转出**：跨能力只传值结构（R16）。
#[cfg(test)]
pub use crate::capabilities::conversation::session::service::assemble::member_face;
pub use crate::capabilities::conversation::session::service::assemble::{
    MemberEnvParts, MemberSpec, ProxySpec, RebuildSpec, SessionPorts, SingleSpec, proxy_session,
    rebuild_single, single_session,
};
// 核心核实环境：`collab` 的各关卡按角色装配只读核实工具，装配逻辑留在本能力。
pub use crate::capabilities::conversation::session::service::verify::{VerifySpec, verify_tools};

// 回合引擎：TurnIo（一次回合的 IO 槽位）、两类回合结果与轮循环（由 `service/engine/` 实现）。
pub use crate::capabilities::conversation::session::service::engine::io::{MemberTurn, TurnIo};
#[allow(unused_imports)]
pub use crate::capabilities::conversation::session::service::engine::round::{
    LineSink, Round, build_round_lines, converse_with, verb_tag,
};
// 核心操作回路 + 原生回灌消息：`service.rs` 实现（它驱动 IO，不是纯派生）。
#[allow(unused_imports)]
pub use crate::capabilities::conversation::session::service::{
    Persister, core_operation, persist_events, record_events, reply_msgs, tool_result_body,
};
// 会话树派生：父子关系、工作根、配置归属、树级装配 meta 与 guest 作用域（只读 `History`）。
#[allow(unused_imports)]
pub use crate::capabilities::conversation::session::service::tree::{
    config_owner_of, flat_modules, guest_lease, guest_lease_for, guest_scope, role_of,
    session_kind_of, subtree_of, tree_meta, tree_meta_with, tree_order, work_root, work_root_of,
};

/// 目的：落盘会话的**队列面**：呈现层经 runtime 的队列代理调它（列表 / 打开 / 删除）。
///   "在世会话 × 历史的并集"（`SessionView`）**不在本面里**：那要同时认识会话中心与历史，
///   归会话中心（`runtime::api::WorkOps::session_views`）。
pub trait HistoryOps: Send + Sync {
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}

/// 目的：会话的**直连面**（`service.rs` 实现）：别的能力要造会话、追流水、读元信息、删会话，走这里；
///   出站端口 `HistoryStore`（目录布局与 append-only 文件格式）**只由它持有**（R12）。
///   它与 `HistoryOps` 的分工是**接收者不同**，不是重复：`HistoryOps` 由队列代理（`Handle`）实现、
///   面向呈现层；`History` 由能力自己实现、面向别的能力。这里的四个写操作（create / save_meta /
///   append / delete）**不开放给呈现层**——呈现层要写就经协调业务的用例。
///   这一面刻意与端口**一一对应**：会话落盘没有别的不变式可编排（追加原子性、元信息唯一真相
///   已在 `domain/history.rs` 与 store 契约里），它的价值是**唯一持有者**（R12），不是新增逻辑。
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
