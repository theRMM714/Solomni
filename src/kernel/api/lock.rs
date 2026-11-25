//! 目的：文件锁的对外面——跨进程互斥、超时与一问即答。
//! 管：`lock_within` / `try_lock` / `held`。
//! 不管：锁文件放在哪、锁名怎么起（在调用方）。
//! 联动：装配层与容量侧的锁经本面取；内核内部直接取 `detail::lock`。

pub use crate::kernel::detail::lock::{held, lock_within, try_lock};
