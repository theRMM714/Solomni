//! 机制型业务的**对外面**：共享事实（`SessionId` / `Tier` / 默认预算）与纯机制
//! （`slash` 的对外书写形式、生成中作业的取消表 `JobRegistry`）。
//!
//! 机制端口（`Log` / `HostProbe`）在 `ports`：它们是全项目共享的机制接口（R12 的例外）。

pub use crate::kernel::domain::approvals::{
    Approval, ApprovalCtx, ApprovalRegistry, ApprovalRequest,
};
pub use crate::kernel::domain::jobs::JobRegistry;
pub use crate::kernel::domain::path::slash;
pub use crate::kernel::domain::types::{SessionId, Tier, ToolOutcome, DEFAULT_LLM_TIMEOUT_SECS};
