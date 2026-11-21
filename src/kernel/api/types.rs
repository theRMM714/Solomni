//! 目的：共享形状的对外面——会话号、档位、执行规格、工具结果、提问与默认预算。
//! 管：`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `JobState` / `Ask` / `AskOutcome` / `DEFAULT_LLM_TIMEOUT_SECS` / `valid_capability`。
//! 不管：这些形状的含义演化（在 `contracts/types`）。
//! 联动：能力侧经本面取类型；内核内部直接取 `contracts::types`。

pub use crate::kernel::contracts::types::{
    Ask, AskOutcome, DEFAULT_LLM_TIMEOUT_SECS, ExecSpec, JobState, SessionId, Tier, ToolOutcome,
    valid_capability,
};
