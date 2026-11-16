//! 目的：turn 能力的入站契约：回合状态的类型与访问器唯一出口。
//! 管：重导出 `service::state` 的 `Session` / `TurnState`、`service::session` 的 `Turn`，以及 `service::assemble` 的装配入口与参数对象。
//! 不管：状态实现（`service/state.rs`）；谁在什么时候放取会话（`conductor` 的用例）。
//! 联动：`conductor` 与 `src/tests/turn.rs` 只经本模块取用。

/// 目的：装配用的三类参数对象：能力面（`SessionDeps`）、一席事实（`MemberSpec`）、单 agent 一场事实（`SingleSpec`）。
#[allow(unused_imports)]
pub use crate::capabilities::turn::service::assemble::{MemberSpec, SessionDeps, SingleSpec};
/// 目的：一席工具面与单 agent 会话对象的装配入口（`conductor` 取完事实后转调这里）。
pub use crate::capabilities::turn::service::assemble::{member_tools, single_session};
/// 目的：回合状态与会话本体的类型出口。
pub use crate::capabilities::turn::service::session::Turn;
/// 目的：在世会话形态与回合状态本身（`conductor` 与测试只经它取用）。
pub use crate::capabilities::turn::service::state::{Session, TurnState};
