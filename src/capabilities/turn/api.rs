//! 目的：turn 能力的入站契约：回合状态的类型与访问器唯一出口。
//! 管：重导出 `service::state` 的 `Session` / `TurnState`。
//! 不管：状态实现（`service/state.rs`）；谁在什么时候放取会话（`conductor` 的用例）。
//! 联动：`conductor` 与 `src/tests/turn.rs` 只经本模块取用。

pub use crate::capabilities::turn::service::state::{Session, TurnState};
