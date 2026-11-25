//! 目的：产品目录布局的对外面——数据根 `data/` 下的设置、台账、日志、会话、镜像、运行包在哪。
//! 管：`domain/layout` 的常量与路径函数。
//! 不管：目录怎么建、怎么回收（在存储与运行包侧）。
//! 联动：能力侧与装配层经本面取；内核内部直接取 `domain::layout`。
//!   整模块重导出，保住 `api::layout::home(…)` 的书写。

pub use crate::kernel::domain::layout::*;
