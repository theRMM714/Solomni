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
pub use crate::kernel::detail::process::{FenceTiming, take_fence_prep};
// 外部工作区的机器级一次性只读授权/撤权：装配层在绑定与解绑时调用（契约出口，消费点由装配层接）。
// 暂未接上时显式放行未用导入，不把出口从契约里删掉。
#[allow(unused_imports)]
pub use crate::kernel::detail::confine::{
    MachineGrantReport, ensure_workspace_grant, revoke_machine_grants,
};
// 视图盘是内核对外契约的一部分：装配层与守门进程按需取用，bin crate 内暂时不会每个都调用到。
// 对未用到的重导出显式放行，不把它们从出口删掉——出口少一个，契约就悄悄漂了。
#[cfg(windows)]
#[allow(unused_imports)]
pub use crate::kernel::detail::view_drive::{
    ViewDrive, active_view, attach_view_drive, ensure_view_drive, host_of, release_view_drive,
    sweep_view_drive, view_codec, view_of,
};

/// 目的：非 Windows 的路径编解码出口——没有视图盘，路径怎么写就怎么用（恒等）。
/// 约束：只有 `view_codec` 被其它平台共用（沙箱与环境块渲染），盘符生命周期只归 Windows。
#[cfg(not(windows))]
pub fn view_codec() -> Option<std::sync::Arc<dyn crate::kernel::ports::PathCodec>> {
    None
}
pub use crate::kernel::domain::fence::{SandboxMode, SandboxRequirements, call_lease};
pub use crate::kernel::domain::jobs::JobRegistry;
pub use crate::kernel::domain::path::{is_unc, native, slash, unc_parts};
