//! 目的：进程账的对外面——当前时间戳这一类跨平台的纯读。
//! 管：`unix_seconds`。
//! 不管：账本身的登记与回收（只在内核内部，见 `detail/procs`）。
//! 联动：单元侧经本面取；内核内部直接取 `detail::procs`。

pub use crate::kernel::detail::procs::unix_seconds;
