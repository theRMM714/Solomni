//! 目的：工具结果的续读办法——按工具种类挑「怎么接着取」，预算与截断本身在 kernel。
//! 管：`continuation_for`（模块工具与 shell 属执行类，文件类可按 offset/limit 接着读）与 kernel 预算面的重导出。
//! 不管：截断与说明（`kernel::api::budget`）；各工具自己的分页（`systool`）。
//! 联动：`tools::api` 从这里重导出 `ResultBudget` / `cap_result_with` / `continuation_for`，调用点不必改道。

pub use crate::kernel::api::budget::{Continuation, ResultBudget, cap_result_with};

/// 目的：按「模块 id + 工具名」判这次调用怎么续读：模块工具与 shell 属执行类，其余按文件类。
pub fn continuation_for(module: &str, tool: &str) -> Continuation {
    if !module.is_empty() || crate::units::business::tools::api::is_exec(tool) {
        Continuation::Command
    } else {
        Continuation::Read
    }
}
