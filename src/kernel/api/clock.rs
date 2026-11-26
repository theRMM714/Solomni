//! 目的：时间的对外面——缺省命名、呈现要的"此刻本地几点"，与记时刻用的时间戳。
//! 管：`local_stamp`（到分钟流水戳）、`now_secs` / `now_nanos`（自 Unix 纪元的秒与纳秒）。
//! 不管：取时的平台实现（在内核内部，见 `detail/clock`）；日期排版与耗时（时长用 `std::time`）。
//! 联动：核心建会话派生缺省名时经本面取；domain 记时间戳也只经本面（`structure/capability` 判）。

pub use crate::kernel::detail::clock::{local_stamp, now_nanos, now_secs};
