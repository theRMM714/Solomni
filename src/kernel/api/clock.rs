//! 目的：本地时间的对外面——缺省命名与呈现需要的"此刻本地几点"。
//! 管：`local_stamp`（到分钟流水戳）。
//! 不管：取时的平台实现（在内核内部，见 `detail/clock`）；日期排版与耗时。
//! 联动：核心建会话派生缺省名时经本面取；内核内部直接取 `detail::clock`。

pub use crate::kernel::detail::clock::local_stamp;
