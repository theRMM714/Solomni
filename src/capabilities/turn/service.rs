//! 目的：turn 的状态与访问器分区声明。
//! 管：`state`（在世会话表、运行态、裁决队、shell 槽）、`session`（会话本体 `Turn`）、`assemble`（一席工具面与单 agent 会话对象的装配）。
//! 不管：生成驱动与落盘。
//! 联动：由本能力的 `api` 重导出。

pub mod assemble;
pub mod session;
pub mod state;
