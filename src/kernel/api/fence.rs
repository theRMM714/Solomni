//! 目的：围栏的对外面——什么该被围住（描述符）与围栏执行的宽严口径（沙箱策略）。
//! 管：围栏意图（`FencePlan` / `Grant` / `Access` / `Reach` / `NetPolicy` / `FenceBlocked` / `FenceRunPolicy`）与沙箱策略（`SandboxMode` / `SandboxRequirements` / `call_lease`）。
//! 不管：描述符的构造与判据（在 `contracts/fence` 与 `domain/fence`）；围栏的执行（在 `detail/confine`）。
//! 联动：能力侧经本面取契约；内核内部直接取 `contracts::fence` / `domain::fence`，不绕本面。

// 围栏意图是内核对外契约的一部分：
// 部分条目眼下只有内核内部与测试用（内部直取机制），不在出口删掉——出口少一个，契约就悄悄漂了。
#[allow(unused_imports)]
pub use crate::kernel::contracts::fence::{
    Access, FenceBlocked, FencePlan, FenceRunPolicy, Grant, NetPolicy, Reach,
};
pub use crate::kernel::domain::fence::{SandboxMode, SandboxRequirements, call_lease};
