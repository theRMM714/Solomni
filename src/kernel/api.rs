//! 目的：地基库的对外面——共享形状与纯机制的出口。
//! 管：共享形状（`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `Ask` / 默认预算 / 围栏描述符 / `RunOutcome`）与纯机制（`slash` 的对外书写形式、`native` 的前缀还原、落点身份与网络路径判定、本地文件戳、围栏准备的耗时交棒、生成中作业的取消表 `JobRegistry`）。
//! 不管：这些形状与机制自身的实现（在 `contracts/` 与 `detail/`）；机制端口的定义（在 `ports`）。
//! 联动：消费方是核心（`src/capabilities/conductor/api/`）与各能力；端口见 `src/kernel/ports.rs`。

pub use crate::kernel::contracts::fence::{
    FENCE_BOUNDARY_MARK, FenceBlocked, FencePart, FenceRunPolicy, FenceSpec,
};
pub use crate::kernel::contracts::run::RunOutcome;
pub use crate::kernel::contracts::types::{
    Ask, AskOutcome, DEFAULT_LLM_TIMEOUT_SECS, ExecSpec, SessionId, Tier, ToolOutcome,
    valid_capability,
};
pub use crate::kernel::detail::host_probe::{FileStamp, file_stamp};
pub use crate::kernel::detail::locus::{is_network_path, locus_of, place_contains};
pub use crate::kernel::detail::process::take_fence_prep;
pub use crate::kernel::domain::fence::{SandboxMode, SandboxRequirements, call_lease};
pub use crate::kernel::domain::jobs::JobRegistry;
pub use crate::kernel::domain::path::{is_unc, native, slash, unc_parts};
