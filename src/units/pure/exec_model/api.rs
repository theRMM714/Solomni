//! 目的：执行与作业模型的入站面——别的单元与呈现层只准用这里。
//! 管：重导出 `Side` / `TargetSpec` / `JobState`。
//! 不管：这些类型怎么被使用（各自的端口与业务流程在环 2 / 环 3）。
//! 联动：sandbox、vm、provision、runtime、maintenance、system、presentation 都经这里取类型。

#[allow(unused_imports)] // 入站面：其它单元与测试都经这里取类型
pub use crate::units::pure::exec_model::domain::exec_selection::{Side, TargetSpec};
#[allow(unused_imports)] // 入站面：进度通道的消费方都经这里取类型
pub use crate::units::pure::exec_model::domain::job::JobState;
