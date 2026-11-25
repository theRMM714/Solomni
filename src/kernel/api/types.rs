//! 目的：共享形状的对外面——会话号、工具结果、提问与默认预算。
//! 管：`SessionId` / `ToolOutcome` / `Ask` / `AskOutcome` / `DEFAULT_LLM_TIMEOUT_SECS` / `valid_capability`。
//! 不管：这些形状的含义演化（在 `contracts/types`）；执行与作业模型（归 units::pure::exec_model）。
//! 联动：内核内部直接取 `contracts::types`；执行档位与进度不在这里。

pub use crate::kernel::contracts::types::{
    Ask, AskOutcome, DEFAULT_LLM_TIMEOUT_SECS, SessionId, ToolOutcome, valid_capability,
};
