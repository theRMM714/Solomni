//! 目的：机制型业务的对外面——共享事实与纯机制的出口。
//! 管：共享事实（`SessionId` / `Tier` / `ToolOutcome` / `Ask` / 默认预算）与纯机制（`slash` 的对外书写形式、生成中作业的取消表 `JobRegistry`）。
//! 不管：这些事实与机制自身的实现（在 `domain/`）；机制端口的定义（在 `ports`）。
//! 联动：消费方是核心（`src/capabilities/conductor/api/`）与各能力；端口见 `src/kernel/ports.rs`。

pub use crate::kernel::domain::jobs::JobRegistry;
pub use crate::kernel::domain::path::slash;
pub use crate::kernel::domain::types::{
    Ask, SessionId, Tier, ToolOutcome, DEFAULT_LLM_TIMEOUT_SECS,
};
