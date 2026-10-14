//! 目的：依赖补齐能力的入站契约——状态、作业快照与两条动作；DTO 是呈现层序列化的形状。
//! 管：Provision trait 与 DepStatus / JobState DTO、未接入的 NoProvision。
//! 不管：清单怎么解析、怎么下载（在 service）。
//! 联动：Ops.provision 持它；Web 的 setup 路由与 CLI 探针都经它。

use serde::Serialize;

/// 目的：一条依赖在本机的状态（给界面看：缺什么、装到哪、从哪来）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DepStatus {
    pub id: String,
    pub present: bool,
    pub install: String,
    pub probe: String,
    pub source: String,
    pub url: Option<String>,
    pub size_mb: u64,
    pub note: String,
}

/// 目的：一次补齐作业的状态快照。
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct JobState {
    pub running: bool,
    pub current: String,
    /// 目的：进度行的**单调序号**（前端按它增量取；`lines` 只留最近若干行，序号不会回退）。
    pub seq: u64,
    pub lines: Vec<String>,
    pub error: Option<String>,
}

/// 目的：依赖补齐的入站用例面——状态、作业快照、起作业、取消。
pub trait Provision: Send + Sync {
    /// 目的：本机依赖状态（缺什么、装到哪、从哪来）。
    fn status(&self) -> Result<Vec<DepStatus>, String>;
    /// 目的：当前作业快照。
    fn job(&self) -> JobState;
    /// 目的：起一个补齐作业（ids 空 = 本机全部）；已有作业在跑则拒绝。
    fn start(&self, ids: Vec<String>) -> Result<(), String>;
    /// 目的：请求取消当前作业。
    fn cancel(&self);
}

/// 目的：未接入的补齐面——组合根未注入时的默认值；调用如实失败，不假装可用。
pub struct NoProvision;

impl Provision for NoProvision {
    fn status(&self) -> Result<Vec<DepStatus>, String> {
        Err("依赖补齐未接入".to_string())
    }
    fn job(&self) -> JobState {
        JobState::default()
    }
    fn start(&self, _ids: Vec<String>) -> Result<(), String> {
        Err("依赖补齐未接入".to_string())
    }
    fn cancel(&self) {}
}
