//! 目的：一条 agent 会话的运行时——会话状态与簿记、成员工具面与面装配、回合引擎、核心操作回路。
//! 管：`api`（入站面）、`domain`（会话对象、工具面、面装配、回档算术、消息回灌）、`engine`（回合入口、轮循环、工具循环、核心操作）。
//! 不管：会话的身份与声明（在 `session_model`）；历史落盘、裁决队列、流程面、会话树（在会话单元）；模型与工具机制（在各自端口后）。
//! 联动：会话单元装配并驱动它；turn / work / collab / runtime 经 `api.rs` 取运行时事实；契约见 docs/agentloop/README.md。

pub mod api;
pub mod domain;
pub mod engine;
