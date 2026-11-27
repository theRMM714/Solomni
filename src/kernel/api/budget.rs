//! 目的：工具回执预算的对外面——单条上限、整批收紧与按预算截断。
//! 管：`ResultBudget`（`full` / `for_batch` / 三个判定）、`Continuation`（续读办法的两类）、`cap_result_with`。
//! 不管：按工具种类挑续读办法（`tools::api::continuation_for`）；各工具自己的分页。
//! 联动：`tools` 的执行面与 `agentloop` 的工具回合都经本面收结果；内核内部直接取 `domain::budget`。

pub use crate::kernel::domain::budget::{
    Continuation, MAX_RESULT_CHARS, ResultBudget, cap_result_with,
};
