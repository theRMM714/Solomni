//! 目的：宿主文件戳的对外面——判断「这份产物还是不是当初那份」。
//! 管：`FileStamp` / `file_stamp`。
//! 不管：戳怎么比较（在调用方的判据里）。
//! 联动：能力侧经本面取；内核内部直接取 `detail::host_probe`。

pub use crate::kernel::detail::host_probe::{FileStamp, file_stamp};
