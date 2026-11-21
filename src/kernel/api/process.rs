//! 目的：进程树的对外面——杀树、围栏准备的耗时交棒。
//! 管：`kill_tree`（crate 内）、`FenceTiming` / `take_fence_prep`。
//! 不管：进程怎么起、围栏怎么落（在 `detail/process` 与 `detail/confine`）。
//! 联动：能力侧与守门进程经本面取；内核内部直接取 `detail::process`。

pub(crate) use crate::kernel::detail::process::kill_tree;
pub use crate::kernel::detail::process::{FenceTiming, take_fence_prep};
