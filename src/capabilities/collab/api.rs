//! 目的：协作能力的入站面——对外导出的用例、会话对象与回合结果类型。
//! 管：CollabSession、AfterTurn 与 MemberTurn 的再导出；driver 的五个自由函数（say / dispatch_task / discussion_turn / continue_reply / compact_session）与 collab_state::tool_runs。
//! 不管：实现（在 service/ 与 domain/）；其它能力的内部；呈现层的渲染与布局。
//! 联动：其它能力与呈现层只经本面使用协作；实现见 src/capabilities/collab/service/，状态派生见 src/capabilities/collab/domain/collab_state.rs。

pub use crate::capabilities::collab::domain::collab_state::tool_runs;
pub use crate::capabilities::collab::service::collab::CollabSession;
pub use crate::capabilities::collab::service::discussion::{AfterTurn, MemberTurn};
pub use crate::capabilities::collab::service::driver::{
    compact_session, continue_reply, discussion_turn, dispatch_task, say,
};
