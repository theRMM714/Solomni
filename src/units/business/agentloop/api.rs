//! 目的：运行时的入站面——其它单元与呈现层只准用这里（不许碰 `domain` / `engine`）。
//! 管：重导出会话对象与回合事实、面装配与整场装配、核实环境、回档算术、消息回灌与核心操作回路。
//! 不管：`domain` / `engine` 的内部实现；历史落盘、裁决队列与会话树（在会话单元）；端口实现（在各自机制单元）。
//! 联动：会话单元的装配与用例、turn 的生成流程、work 与 collab 的调度、runtime 的核心操作都从这里取；契约见 docs/agentloop/README.md。

/// 目的：测试用——按一席的来源算工具面 id（生产路径只有 assemble）。
#[cfg(test)]
pub use crate::units::business::agentloop::domain::assemble::seat_face;
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::assemble::{
    FaceInput, SeatFace, assemble, face_of, service_ops_of, serviced_tools,
};
/// 目的：只取装配出的工具面值；生产路径都直接建会话，所以它只在测试里编译。
#[cfg(test)]
pub use crate::units::business::agentloop::domain::assembly::member_face;
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::assembly::{
    MemberEnvParts, MemberSpec, ProxySpec, RebuildSpec, SingleSpec, proxy_session, rebuild_single,
    single_session,
};
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::member_tools::{
    CoreVerify, ModuleTools, ServiceOp, ShellLive, ShellSlot, ToolFace, last_shell,
    service_ops_for_modules, tool_table,
};
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::ports::SessionPorts;
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::reply::{reply_msgs, tool_result_body};
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::rewind::{
    ArchiveMark, align_keep, archive_label, cut_before_line, folded, last_line_within, last_usage,
    line_reply_of, max_reply, next_line_id, rewind_marks, truncate_events, turn_of_line,
};
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::session::{
    AgentSession, CompactChatFactory, CompactChatSpec, CompactionPlan, FRESHNESS_MIN_SAVED_CHARS,
    FreshnessStats, ReadAt, SessionParams, TurnRun, delta_visible, expired_note, freshness_view,
    keep_whole_replies, stream_piece, summary_message, unique_work_name,
};
/// 目的：测试用——直接造一个来源，验「它按什么条件交 id」；生产路径由会话类型挂来源。
#[cfg(test)]
pub use crate::units::business::agentloop::domain::sources::WorkspaceSource;
#[allow(unused_imports)]
pub use crate::units::business::agentloop::domain::verify::{VerifySpec, verify_tools};
#[allow(unused_imports)]
pub use crate::units::business::agentloop::engine::core_op::core_operation;
#[allow(unused_imports)]
pub use crate::units::business::agentloop::engine::io::{CompactedTurn, MemberTurn, TurnIo};
#[allow(unused_imports)]
pub use crate::units::business::agentloop::engine::round::{
    LineSink, Round, RoundSink, build_round_lines, converse_with, verb_tag,
};

/// 目的：测试直调工具批执行器；生产路径只经回合引擎，不从这里取。
#[cfg(test)]
pub(crate) use crate::units::business::agentloop::engine::tools::run_batch;
