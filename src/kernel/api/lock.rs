//! 目的：文件锁的对外面——跨进程互斥与超时。
//! 管：`lock_within`。
//! 不管：锁文件放在哪、锁名怎么起（在调用方）。
//! 联动：装配层与容量侧的锁经本面取；内核内部直接取 `detail::lock`。

pub use crate::kernel::detail::lock::lock_within;
