//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain`）。

pub use crate::capabilities::collab::domain::collab_state::tool_runs;
pub use crate::capabilities::collab::service::collab::CollabSession;
pub use crate::capabilities::collab::service::engine::{AfterTurn, MemberTurn};
