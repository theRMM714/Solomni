//! 目的：回执说明的对外面——一条说明、严重度与稳定码词表。
//! 管：`Note` / `Severity` / `codes`。
//! 不管：说明怎么渲染成文本（在 `detail/i18n`）；说明怎么进入回执（在 `contracts/run`）。
//! 联动：能力侧经本面取类型；内核内部直接取 `contracts::note`。

pub use crate::kernel::contracts::note::{Note, Severity, codes};
