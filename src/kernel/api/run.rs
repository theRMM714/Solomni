//! 目的：一次执行结果的对外面——进程的产出、说明与耗时。
//! 管：`RunOutcome`。
//! 不管：结果怎么采集（在 `detail/process`）。
//! 联动：能力侧经本面取类型；内核内部直接取 `contracts::run`。

pub use crate::kernel::contracts::run::RunOutcome;
