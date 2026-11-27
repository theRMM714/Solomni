//! 目的：执行环境的会话声明——只写「用哪个策略」和策略自己的参数。
//! 管：`SandboxDecl`（策略 id、定版表、不透明参数）。
//! 不管：参数怎么解释（各自的策略）；执行在哪一侧（`exec_model` 的 `Side`）；会话怎么建（`work`）。
//! 联动：`session_model` 的会话 meta 存它；`Sandboxes` 按 `policy` 找实现。

use crate::units::pure::exec_model::api::TargetSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：缺省策略 id——没有声明执行环境时就在本机跑。
pub const HOST_POLICY: &str = "host";
/// 目的：虚拟机策略 id。
pub const VM_POLICY: &str = "vm";

fn default_policy() -> String {
    HOST_POLICY.to_string()
}

/// 目的：会话对执行环境的声明——形状只由策略 id 与它自己的参数决定。
/// 约束：字段都是数据；解释权在 `SandboxPolicy`，会话不读 `params` 里的键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxDecl {
    /// 目的：策略 id（注册表里的键）。
    #[serde(default = "default_policy")]
    pub policy: String,
    /// 目的：定版表——运行包 id → 版本（策略决定用不用它）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pins: BTreeMap<String, String>,
    /// 目的：策略自己的参数（会话不解释、只透传）；缺省是 `null`。
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

impl Default for SandboxDecl {
    fn default() -> Self {
        SandboxDecl {
            policy: default_policy(),
            pins: BTreeMap::new(),
            params: serde_json::Value::Null,
        }
    }
}

impl SandboxDecl {
    /// 目的：本机策略的声明。
    pub fn host() -> SandboxDecl {
        SandboxDecl::default()
    }

    /// 目的：指定策略 id 的声明（参数仍要调用方填）。
    pub fn of(policy: &str) -> SandboxDecl {
        SandboxDecl {
            policy: policy.to_string(),
            ..SandboxDecl::default()
        }
    }

    /// 目的：参数的字符串值（类型不对按没有处理，由策略自己报）。
    pub fn param_str(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(|v| v.as_str())
    }

    /// 目的：把声明翻成计划要用的选型事实（哪一侧、哪个虚拟根、定哪个版本）。
    /// 参数：`facts` = 策略按这份声明回答的事实。
    /// 约束：只有事实与定版表进来——策略 id 与参数留在声明里，执行计划不需要认识它们。
    pub fn target(&self, facts: &super::facts::SandboxFacts) -> TargetSpec {
        TargetSpec {
            side: facts.side,
            guest: facts.guest.clone(),
            pins: self.pins.clone(),
        }
    }
}
