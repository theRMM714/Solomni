//! 目的：一次外部进程执行的**中性结果**——只陈述成败、轻重、输出与结构化说明，不解释业务含义。
//! 管：`RunOutcome { ok, severity, output, notes }` 这一种形状。
//! 不管：工具回执的业务映射（在 `units/tools`）；进程怎么跑（在 `kernel/detail/process.rs`）。
//! 联动：`ProcessRunner` 端口返回它；工具能力把它包成对外的 `ToolOutcome`。

use crate::kernel::contracts::note::{Note, Severity};

/// 目的：一次外部进程执行的事实结果——`ok` = 成功；`severity` 分轻重；`output` 是原始输出；
///   `notes` 是结构化说明（稳定码 + 参数，不带文案）。
/// 约束：它是机制层与业务层之间的中性形状；业务语义由调用方解释，本类型不认识任何工具/围栏词汇。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub ok: bool,
    pub severity: Severity,
    pub output: String,
    pub notes: Vec<Note>,
}

impl RunOutcome {
    /// 目的：按成败造一条没有说明的结果——成功是 `Info`，失败是 `Error`。
    /// 约束：只给测试替身用（生产路径按四个字段各自给值）。
    #[cfg(test)]
    pub fn new(ok: bool, output: impl Into<String>) -> RunOutcome {
        RunOutcome {
            ok,
            severity: if ok { Severity::Info } else { Severity::Error },
            output: output.into(),
            notes: Vec::new(),
        }
    }

    /// 目的：造一条「没执行 / 起不来」的结果（`Error`，无说明文案，说明由调用方补）。
    pub fn failed(output: impl Into<String>) -> RunOutcome {
        RunOutcome {
            ok: false,
            severity: Severity::Error,
            output: output.into(),
            notes: Vec::new(),
        }
    }
}
