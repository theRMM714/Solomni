//! 目的：turn 的入站面——`TurnState` 与 `Session` 的对外重导出。
//! 管：把 state 里的两个类型按 `api` 契约暴露给唯一调用方 `conductor`。
//! 不管：状态的持有与访问器实现（在 service/state.rs）；不碰 IO、不碰用例编排。
//! 联动：`conductor` 的 service 各文件经 `crate::capabilities::turn::api` 取类型。

pub use crate::capabilities::turn::service::state::{Session, TurnState};
