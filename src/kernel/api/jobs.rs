//! 目的：生成中作业取消表的对外面——谁在跑、能不能取消。
//! 管：`JobRegistry`。
//! 不管：取消信号怎么送到进程（在 `detail/process`）。
//! 联动：能力侧经本面取；内核内部直接取 `domain::jobs`。

pub use crate::kernel::domain::jobs::JobRegistry;
