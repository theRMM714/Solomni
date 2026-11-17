//! 目的：协作能力的入站面——对外导出的会话对象与回合处置。
//! 管：CollabSession、AfterTurn 的再导出与 collab_state::tool_runs。
//! 不管：实现（在 service/ 与 domain/）；回合引擎（已迁 session，见 session::service::engine）；其它能力的内部；呈现层的渲染与布局。
//! 联动：其它能力与呈现层只经本面使用协作；实现见 src/capabilities/collab/service/，状态派生见 src/capabilities/collab/domain/collab_state.rs。

pub use crate::capabilities::conversation::collab::domain::collab_state::tool_runs;
pub use crate::capabilities::conversation::collab::service::collab::CollabSession;
pub use crate::capabilities::conversation::collab::service::discussion::AfterTurn;
