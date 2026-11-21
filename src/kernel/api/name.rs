//! 目的：名字规则的对外面——目录名合法性与去重后缀。
//! 管：`validate_dir_name` / `dedup_suffix`。
//! 不管：名字用在哪个业务（由调用方解释）。
//! 联动：能力侧经本面取；内核内部直接取 `domain::name`。

pub use crate::kernel::domain::name::{dedup_suffix, validate_dir_name};
