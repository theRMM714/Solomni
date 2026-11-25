//! 目的：长任务的进度快照——补齐环境与还原本地空间共用同一条进度通道（`seq` + `lines`）。
//! 管：`JobState` 这一个形状。
//! 不管：谁在跑、进度从哪来（provision / maintenance 各自持有作业）；通道怎么传（在各自的端口）。
//! 联动：provision、maintenance、system 与呈现层都经 units::pure::exec_model::api 取它；唯一家判据见 R6。

use serde::Serialize;

/// 目的：长任务的进度快照——补齐环境与还原本地空间共用同一条进度通道（`seq` + `lines`）。
/// 约束：共享形状只有这一份（R6）——两个产生方各自持有作业，形状不各造一份（R15）。
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobState {
    /// 目的：是否还在跑。
    pub running: bool,
    /// 目的：进度行的**单调序号**（前端按它增量取；`lines` 只留最近若干行，序号不会回退）。
    pub seq: u64,
    /// 目的：最近的进度行（更早的被挤掉）。
    pub lines: Vec<String>,
    /// 目的：失败原因（在跑或成功时为空）。
    pub error: Option<String>,
}
