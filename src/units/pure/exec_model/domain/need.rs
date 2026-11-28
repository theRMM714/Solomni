//! 目的：执行环境的「还缺什么、怎么补」——策略自己回答，呈现层照抄（不各自编话、不写死策略名）。
//! 管：`Requirement`（一项前置：事实 + 补法）、`Fix`（补法的类别与载荷）、`Readiness`（一份声明的逐项前置）、`Machine`（与声明无关的机器能力）、`FixStep` 与 `Fixer`（补齐这个动作的端口）。
//! 不管：补什么、怎么下载（`provision` 的清单与作业）、界面怎么排版（`presentation` 只按 `kind` 分支）。
//! 联动：`sandbox` 的策略默认方法返回这里；`vm` 产虚拟机的清单；`provision` 实现 `Fixer`；`work` 与呈现层经 `sandbox::api` 读。

use serde::Serialize;

/// 目的：补法的类别——呈现层据此决定给不给按钮、按钮点了发什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixKind {
    /// 目的：能下载补齐（`deps` 是要装的依赖 id，真相在 `dependencies.yaml`）。
    Download,
    /// 目的：只能改本机设置（工作台不代改系统，只给指引）。
    HostSetting,
    /// 目的：这里补不了（自己解决或换档），`summary` 如实说为什么。
    None,
}

/// 目的：一项前置的补法——**策略与补齐引擎之间唯一跨界的词表**是 `deps` 里的依赖 id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fix {
    /// 目的：补法类别。
    pub kind: FixKind,
    /// 目的：给人看的一句话（没有补法时是"为什么补不了"）。
    pub summary: String,
    /// 目的：`Download` 要装的依赖 id（`dependencies.yaml` 的 id；空 = 不知道该装哪条）。
    pub deps: Vec<String>,
    /// 目的：动手之前要不要用户明确同意（只写项目内、要下载时恒 true）。
    pub consent: bool,
}

impl Fix {
    /// 目的：不用补（前置已满足，或与声明无关的项）。
    pub fn none() -> Fix {
        Fix {
            kind: FixKind::None,
            summary: String::new(),
            deps: Vec::new(),
            consent: false,
        }
    }

    /// 目的：这项在这里补不了——如实说为什么、自己怎么办。
    pub fn blocked(summary: impl Into<String>) -> Fix {
        Fix {
            kind: FixKind::None,
            summary: summary.into(),
            deps: Vec::new(),
            consent: false,
        }
    }

    /// 目的：改本机设置才能解决（工作台只给指引，不代改系统）。
    pub fn host_setting(summary: impl Into<String>) -> Fix {
        Fix {
            kind: FixKind::HostSetting,
            summary: summary.into(),
            deps: Vec::new(),
            consent: false,
        }
    }

    /// 目的：下载补齐这几条依赖（`deps` 为空时按"不在这里补"处理，不给假按钮）。
    pub fn download(summary: impl Into<String>, deps: Vec<String>) -> Fix {
        if deps.is_empty() {
            return Fix::blocked(summary);
        }
        Fix {
            kind: FixKind::Download,
            summary: summary.into(),
            deps,
            consent: true,
        }
    }
}

/// 目的：一项前置要求——事实 + 怎么解决（界面与日志照抄，不各自编话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Requirement {
    /// 目的：稳定短名（机器可读，界面/日志按它分支）。
    pub id: String,
    /// 目的：给人看的名字（界面直接显示，不按 id 编话）。
    pub label: String,
    /// 目的：满没满足。
    pub met: bool,
    /// 目的：现状一句话（满足与不满足都要能读）。
    pub detail: String,
    /// 目的：没满足时怎么补（满足了就是 `Fix::none()`）。
    pub fix: Fix,
}

impl Requirement {
    /// 目的：满足的一项（没有补法）。
    pub fn met(
        id: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) -> Requirement {
        Requirement {
            id: id.into(),
            label: label.into(),
            met: true,
            detail: detail.into(),
            fix: Fix::none(),
        }
    }

    /// 目的：不满足的一项（带补法）。
    pub fn unmet(
        id: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        fix: Fix,
    ) -> Requirement {
        Requirement {
            id: id.into(),
            label: label.into(),
            met: false,
            detail: detail.into(),
            fix,
        }
    }
}

/// 目的：一条声明的逐项前置——空表 = 这一侧没有任何额外前置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Readiness {
    /// 目的：这一侧的全部前置要求（本机侧通常为空）。
    pub requirements: Vec<Requirement>,
}

impl Readiness {
    /// 目的：齐了没有（空表 = 齐了）。
    pub fn ready(&self) -> bool {
        self.requirements.iter().all(|r| r.met)
    }

    /// 目的：缺哪几项（空 = 齐了）：呈现层按它如实说明，不猜。
    pub fn unmet(&self) -> Vec<&Requirement> {
        self.requirements.iter().filter(|r| !r.met).collect()
    }

    /// 目的：缺什么（每项一句"现状"）：日志与拒绝理由用。
    pub fn missing(&self) -> Vec<&str> {
        self.unmet().iter().map(|r| r.detail.as_str()).collect()
    }
}

/// 目的：一个策略的机器能力（与某条会话无关）：界面据此决定这一侧能不能选、缺的怎么补。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Machine {
    /// 目的：策略 id。
    pub policy: String,
    /// 目的：给人看的名字（呈现层直接用，不自己编）。
    pub name: String,
    /// 目的：这台机器上这一侧的全部前置要求。
    pub requirements: Vec<Requirement>,
}

impl Machine {
    /// 目的：没登记这个策略时的空结论（不是"可用"，是一条不满足的前置）。
    pub fn absent(policy: &str) -> Machine {
        Machine {
            policy: policy.to_string(),
            name: String::new(),
            requirements: vec![Requirement::unmet(
                "policy",
                "执行环境策略",
                format!("没有这个执行环境策略：{}", policy),
                Fix::blocked("在组合根登记这个策略，或改用登记过的 id。"),
            )],
        }
    }

    /// 目的：齐了没有（空表 = 齐了）。
    pub fn ready(&self) -> bool {
        self.requirements.iter().all(|r| r.met)
    }

    /// 目的：缺什么（每项一句"现状"）。
    pub fn missing(&self) -> Vec<&str> {
        self.requirements
            .iter()
            .filter(|r| !r.met)
            .map(|r| r.detail.as_str())
            .collect()
    }
}

/// 目的：补齐作业里的一步（下载一条依赖的展示事实）：按钮的确认清单照抄它。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FixStep {
    /// 目的：依赖 id（`dependencies.yaml` 的 id）。
    pub id: String,
    /// 目的：给人看的名字。
    pub label: String,
    /// 目的：下载地址（没有就空）。
    pub url: Option<String>,
    /// 目的：大致体积（MB）。
    pub size_mb: u64,
    /// 目的：一句话说明（装到哪、注意什么）。
    pub note: String,
}

/// 目的：补齐端口——策略决定"缺什么"，这个端口只负责"按依赖 id 补"。
/// 约束：实现放在 `provision`（组合根注入策略），策略本身不依赖补齐引擎。
pub trait Fixer: Send + Sync {
    /// 目的：把要装的依赖 id 展开成确认清单；id 不在清单里时如实报错。
    fn plan(&self, ids: &[String]) -> Result<Vec<FixStep>, String>;
    /// 目的：开始装（只写项目内）；进度由呈现层读 `/api/setup/status`。
    fn start(&self, ids: Vec<String>) -> Result<(), String>;
}
