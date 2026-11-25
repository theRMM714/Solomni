//! 目的：执行与作业模型的入站面——别的单元与呈现层只准用这里。
//! 管：重导出 `Tier` / `ExecSpec` / `JobState`。
//! 不管：这些类型怎么被使用（各自的端口与业务流程在环 2 / 环 3）。
//! 联动：vm、provision、runtime、maintenance、system、presentation 都经这里取类型。

#[allow(unused_imports)] // 入站面：其它单元与测试都经这里取类型
pub use crate::units::exec_model::domain::exec_selection::{ExecSpec, Tier};
#[allow(unused_imports)] // 入站面：进度通道的消费方都经这里取类型
pub use crate::units::exec_model::domain::job::JobState;
