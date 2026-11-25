//! 目的：路径书写的对外面——斜杠形式、本机原生前缀还原、UNC 判定。
//! 管：`slash` / `native` / `is_unc` / `unc_parts`。
//! 不管：落点身份（那是 `api::locus` 的事）。
//! 联动：单元侧经本面取；内核内部直接取 `domain::path`。

pub use crate::kernel::domain::path::{is_unc, native, slash, unc_parts};
