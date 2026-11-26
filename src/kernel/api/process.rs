//! 目的：进程的对外面——起一个长驻子进程、杀树、围栏准备的耗时交棒。
//! 管：`spawn`（`Spawn` → `Daemon`）、`FenceTiming` / `take_fence_prep`。
//! 不管：命令行怎么拼、就绪判据与停机语义（在单元）；父死子亡与账本的实现（在 `detail/procs`）。
//! 联动：单元侧与守门进程经本面取；内核内部直接取 `detail` 下的机制。

pub use crate::kernel::detail::process::{FenceTiming, take_fence_prep};
pub use crate::kernel::detail::procs::daemon::Daemon;
pub use crate::kernel::detail::procs::spawn::{Spawn, Stream, spawn};
