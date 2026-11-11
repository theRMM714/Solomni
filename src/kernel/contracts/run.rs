//! 目的：一次外部进程执行的**中性结果**——只陈述成功与输出，不解释业务含义。
//! 管：`RunOutcome { ok, output }` 这一种形状。
//! 不管：工具回执的业务映射（在 `capabilities/tools`）；进程怎么跑（在 `kernel/detail/process.rs`）。
//! 联动：`ProcessRunner` 端口返回它；工具能力把它包成对外的 `ToolOutcome`。

/// 目的：一次外部进程执行的事实结果——`ok` = 成功；`output` 已按执行者的规则截断。
/// 约束：它是机制层与业务层之间的中性形状；业务语义由调用方解释，本类型不认识任何工具/围栏词汇。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub ok: bool,
    pub output: String,
}
