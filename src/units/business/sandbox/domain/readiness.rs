//! 目的：执行环境的就绪判定——策略自己回答「这台机器能不能跑、这条声明缺什么、自带什么运行能力」。
//! 管：`Requirement`（一项前置：事实 + 怎么补）、`Readiness`（一份声明的逐项前置）、`Machine`（与声明无关的机器能力）。
//! 不管：缺什么怎么办（补齐面）、界面怎么排版（呈现层照抄 id / met / detail / how）。
//! 联动：`SandboxPolicy` 的三个默认方法返回这里；`SandboxPolicies` 汇总给 `work` 与呈现层。

use serde::Serialize;

/// 目的：一项前置要求——事实 + 怎么解决（界面与日志照抄，不各自编话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Requirement {
    /// 目的：稳定短名（机器可读，界面/日志按它分支）。
    pub id: String,
    /// 目的：满没满足。
    pub met: bool,
    /// 目的：现状一句话（满足与不满足都要能读）。
    pub detail: String,
    /// 目的：没满足时怎么解决（满足了就是空）。
    pub how: String,
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

/// 目的：一个策略的机器能力（与某条会话无关）：界面据此决定这一侧能不能选。
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
            requirements: vec![Requirement {
                id: "policy".to_string(),
                met: false,
                detail: format!("没有这个执行环境策略：{}", policy),
                how: "在组合根登记这个策略，或改用登记过的 id。".to_string(),
            }],
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
