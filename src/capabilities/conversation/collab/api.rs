//! 目的：协作能力的入站面——对外导出的会话对象与回合处置。
//! 管：CollabSession、机制词汇（事件 / 关卡 / 选项 id）与 collab_state::tool_runs 的再导出。
//! 不管：实现（在 service/ 与 domain/）；回合引擎（已迁 session，见 session::service::engine）；其它能力的内部；呈现层的渲染与布局。
//! 联动：其它能力与呈现层只经本面使用协作；实现见 src/capabilities/conversation/collab/service/，状态派生见 src/capabilities/conversation/collab/domain/collab_state.rs。

pub use crate::capabilities::conversation::collab::domain::collab_state::tool_runs;
/// 目的：检查项的形状（Review 载荷的解释）；呈现按载荷 JSON 渲染，生产侧不直接引用。
#[cfg(test)]
pub use crate::capabilities::conversation::collab::domain::events::CheckView;
pub use crate::capabilities::conversation::collab::domain::events::CollabEvent;
/// 目的：待决项的类型化形态（关卡的解释）；驱动与呈现都按 `Gate` 走，只有测试与状态推导用它。
#[cfg(test)]
pub use crate::capabilities::conversation::collab::domain::gates::Pending;
// 选项 id：机制对外的作答词汇（驱动按字符串答、测试与前端按同一份字符串对），
// 生产侧只用到其中一部分，故不逐个当作未用告警。
#[allow(unused_imports)]
pub use crate::capabilities::conversation::collab::domain::gates::{
    OPT_ASK_REPLY, OPT_BEGIN, OPT_BEGIN_ALLOW, OPT_NODE_REWORK, OPT_NODE_SAY, OPT_PLAN_SAY,
    OPT_PLAN_START, OPT_SLATE_CANCEL, OPT_SLATE_CONFIRM,
};
pub use crate::capabilities::conversation::collab::service::collab::CollabSession;
pub use crate::capabilities::conversation::collab::service::collab::Phase;
