//! 目的：策略回答的事实与注册项。
//! 管：`SandboxFacts`（这一席在哪一侧、客机是什么）、`PolicyDecl`（策略的自我介绍）。
//! 不管：事实怎么算（各自的策略）；就绪之后怎么装（`work`）。
//! 联动：`service::Sandboxes` 聚成注册表；`work` 与呈现层按它取展示与判定。

use crate::units::business::sandbox::domain::decl::HOST_POLICY;
use crate::units::pure::exec_model::api::Side;
use crate::units::pure::registry_model::api::Param;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：一个会话的执行环境事实——会话与工具看这个，不看策略是怎么实现的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxFacts {
    /// 目的：策略 id。
    pub policy: String,
    /// 目的：包安装侧与平台目录名的那一侧。
    pub side: Side,
    /// 目的：客机镜像 id（本机策略没有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest: Option<String>,
    /// 目的：客机 OS（本机策略没有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_os: Option<String>,
    /// 目的：客机架构（本机策略没有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_arch: Option<String>,
}

impl Default for SandboxFacts {
    fn default() -> Self {
        SandboxFacts::host()
    }
}

impl SandboxFacts {
    /// 目的：本机事实。
    pub fn host() -> SandboxFacts {
        SandboxFacts {
            policy: HOST_POLICY.to_string(),
            side: Side::Host,
            guest: None,
            guest_os: None,
            guest_arch: None,
        }
    }

    /// 目的：客机侧事实——虚拟根名已知，OS 与架构等策略解析镜像时补（`guest_os` / `guest_arch` 留空）。
    #[cfg(test)]
    pub fn guest(guest: &str) -> SandboxFacts {
        SandboxFacts {
            policy: super::decl::VM_POLICY.to_string(),
            side: Side::Guest,
            guest: Some(guest.to_string()),
            guest_os: None,
            guest_arch: None,
        }
    }

    /// 目的：是不是在客机上跑（决定包安装侧与平台目录）。
    pub fn on_guest(&self) -> bool {
        self.side == Side::Guest
    }
}

/// 目的：策略的自我介绍（呈现层与建会话的参数校验读它）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicyDecl {
    /// 目的：注册表里的键，也是会话声明里写的值。
    pub id: String,
    /// 目的：给人看的名字。
    pub name: String,
    /// 目的：一句话说明（选择器里显示）。
    pub hint: String,
    /// 目的：参数声明（与工具、命令同一套 `Param`）。
    pub params: Option<BTreeMap<String, Param>>,
    /// 目的：参数名 → **取值域 id**：这个参数的候选值由别处的事实给（如虚拟根清单），
    ///   声明方不自己编清单；呈现层按解析后的 `Param.values` 画下拉。
    /// 约束：解析归工作台（它手上有那些事实）；认不出的域 = 空表（退化成手填，不编造）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, String>,
}
