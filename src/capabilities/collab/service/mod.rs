//! 本能力的**用例与编排**：讨论/执行/验收引擎、协作会话状态机。
//!
//! 它们驱动 IO（模型调用、工具执行、落盘），所以按 refactor-plan §2.3 / §3.7 落 `service/`；
//! `domain/` 只留纯派生（`collab_state`）。对外仍然只经 `api`。

pub mod collab;
pub mod engine;
