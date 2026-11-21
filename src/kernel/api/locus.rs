//! 目的：落点身份的对外面——这个路径在哪（本机盘、网络盘还是别处）。
//! 管：`locus_of` / `place_contains` / `is_network_path`。
//! 不管：落点判定的平台实现（在 `detail/locus`）。
//! 联动：能力侧经本面取；内核内部直接取 `detail::locus`。

pub use crate::kernel::detail::locus::{is_network_path, locus_of, place_contains};
