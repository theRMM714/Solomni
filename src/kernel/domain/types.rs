//! 目的：跨业务共享的事实类型。
//! 管：只没有领域逻辑的事实——`SessionId` / `Tier` / `ToolOutcome` / 默认预算。
//! 不管：带领域逻辑的类型（归各自能力）；各业务自造一份 DTO 副本（R6 禁止）。
//! 联动：由 `src/kernel/api.rs` 重导出；判据见 ARCHITECTURE.md 的「硬要求清单」R6。

use serde::{Deserialize, Serialize};

/// 目的：执行档位——本机 = 直接在宿主上跑；虚拟机 = 整台 guest（不信任 AI 时的可选档）。
/// 约束：事实类型只属于 kernel（R6）——它被登记处与执行计划共享，且不认识任何业务概念。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Host,
    Vm,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Host => "host",
            Tier::Vm => "vm",
        }
    }
}

/// 目的：单次模型调用的默认总预算（秒）。
/// 约束：登记处（设置项）与 `llm` 的出站调用参数共享它；设置项见 `AppSettings::llm_timeout_secs`。
pub const DEFAULT_LLM_TIMEOUT_SECS: u64 = 300;

/// 目的：前端唯一的会话标识 = 工作名（用户的命名，也是落盘目录名）。
pub type SessionId = String;

/// 目的：一次工具调用的事实结果——`ok` = 成功；`output` 已截断（截断规则由各执行者定）。
/// 约束：内置工具、模块工具与核心自有工具共用这同一个形状，谁都不该复制第二份（R6）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub ok: bool,
    pub output: String,
}
