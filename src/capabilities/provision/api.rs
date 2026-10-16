//! 目的：依赖补齐能力的入站契约——状态、虚拟根清单、作业快照与两条动作；DTO 是呈现层序列化的形状。
//! 管：Provision trait 与 DepStatus / GuestBase / JobState DTO、未接入的 NoProvision。
//! 不管：清单怎么解析、怎么下载（在 service）。
//! 联动：Ops.provision 持它；Web 的 setup 路由与 CLI 探针都经它；conductor 的"选虚拟机"经 `guest_bases` 取候选。

use serde::Serialize;

/// 目的：一条依赖在本机的状态（给界面看：缺什么、装到哪、从哪来）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DepStatus {
    pub id: String,
    /// 目的：归属的虚拟根名（只有 guest 类条目有；其余为空）。
    pub guest: Option<String>,
    pub present: bool,
    pub install: String,
    pub probe: String,
    pub source: String,
    pub url: Option<String>,
    pub size_mb: u64,
    pub note: String,
}

/// 目的：清单里的一个**虚拟根**（虚拟机档的候选）——名字 + 本机能不能跑 + 装没装 + 装到哪。
/// 约束：列表里绝大多数信息由清单条目归并而来（一个虚拟根可以由多条条目共同装：镜像 + 运行期编排）；
///   `label` 只给界面显示，取镜像画像的 `id`（缺画像就退名字）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuestBase {
    /// 目的：虚拟根名（清单 `guest` 字段；也是会话 `ExecSpec.guest` 存的值）。
    pub guest: String,
    /// 目的：给人看的显示名（镜像 `guest.yaml` 的 `id`，例如 debian-13-…；缺画像时退名字）。
    pub label: String,
    /// 目的：guest 的操作系统（`guest.yaml` 的 os；缺画像按 linux）。
    pub os: String,
    pub arch: String,
    /// 目的：这个虚拟根**自证过的**运行能力（`guest.yaml` 的 runtimes）：界面据此说明它自带什么。
    pub runtimes: Vec<String>,
    /// 目的：本机的清单条目是否都在场（缺哪条 = 还没装完，`missing` 列出来）。
    pub ready: bool,
    /// 目的：没就绪时缺哪几条（条目 id）；就绪时为空。
    pub missing: Vec<String>,
    /// 目的：装到产品根下的哪一格（相对路径；诊断与日志用，界面按需显示）。
    pub install: String,
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

/// 目的：依赖补齐的入站用例面——状态、虚拟根清单、作业快照、起作业、取消。
pub trait Provision: Send + Sync {
    /// 目的：本机依赖状态（缺什么、装到哪、从哪来）。
    fn status(&self) -> Result<Vec<DepStatus>, String>;
    /// 目的：本机的虚拟根清单（虚拟机档的候选；只列本机平台用得上的那些）。
    /// 约束：只是清单事实 + 是否装齐，不做启动判定（那在 `vm` 的档位判据里）。
    fn guest_bases(&self) -> Result<Vec<GuestBase>, String>;
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
    fn guest_bases(&self) -> Result<Vec<GuestBase>, String> {
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
