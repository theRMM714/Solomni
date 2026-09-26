//! 跨业务共享的**事实类型**：只放没有领域逻辑的。
//! 见 docs/architecture/refactor-plan.md §1.3 R6：事实类型只属于 kernel，禁止各业务复制 DTO。

use serde::{Deserialize, Serialize};

/// 执行档位：本机 = 直接在宿主上跑；虚拟机 = 整台 guest（不信任 AI 时的可选档）。
/// **为什么在 kernel**：它被登记处（新会话的默认档位）与执行计划共享，且不认识任何业务概念；
/// 留在 `exec` 里会让登记处反过来依赖执行能力（`registry ⇄ workspace` 环）。
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

/// 单次模型调用的默认总预算（秒）。见 `AppSettings::llm_timeout_secs`。
/// 登记处（设置项）与 `llm` 的出站调用参数共享它，所以放内核。
pub const DEFAULT_LLM_TIMEOUT_SECS: u64 = 300;

/// 前端唯一的会话标识 = 工作名（用户的命名，也是落盘目录名）。
pub type SessionId = String;
