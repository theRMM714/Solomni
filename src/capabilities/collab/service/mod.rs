//! 本能力的**用例与编排**：讨论/执行/验收引擎、协作会话状态机。
//!
//! 它们驱动 IO（模型调用、工具执行、落盘），所以按 ARCHITECTURE.md §九.3 落 `service/`；
//! `domain/` 只留纯派生（`collab_state`）。对外仍然只经 `api`。

pub mod collab;
pub mod discussion;
pub mod driver;
pub mod pump;
pub mod round;
pub mod slate;
pub mod synthesis;
pub mod tool_loop;
pub mod turn_io;
