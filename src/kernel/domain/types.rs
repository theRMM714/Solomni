//! 目的：跨业务共享的事实类型。
//! 管：只没有领域逻辑的事实——`SessionId` / `Tier` / `ToolOutcome` / `Ask`（请用户裁决的问题）/ 默认预算。
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

    /// 目的：把对外写法解析成档位；缺省（空串）= 本机档，未知值如实报错。
    /// 约束：呈现层与协调业务共用这一处解析，不再各写一份（对外词汇只有 host / vm）。
    pub fn parse(s: &str) -> Result<Tier, String> {
        match s.trim() {
            "" | "host" => Ok(Tier::Host),
            "vm" => Ok(Tier::Vm),
            other => Err(format!("未知执行档位：{}（只接受 host / vm）", other)),
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

/// 目的：一条**请用户裁决**的问题：谁在问（信封）+ 消息三段 + 选项集——通道只管送，不解释内容。
/// 约束：请求方（工具执行层、围栏这类机制）与通道（会话的裁决队）共用这一个形状，所以它属于 kernel（R6）；
///   问什么、有几个选项、每个选项做什么都由请求方实现，通道只把选项 **id** 原样还给请求方。
///   端口见 `crate::kernel::ports::AskUser`；选项 id 与卡片的派生见 docs/session/session-model.md 的「请用户裁决」。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    /// 目的：谁在问——信封第一段（core 核心 / member 某一席 / tools 工具层）。
    pub role: String,
    /// 目的：具体是谁——信封第二段（agent 实例名或机制名）。
    pub name: String,
    /// 目的：标题（一句话说清这一问是什么）。
    pub title: String,
    /// 目的：正文（为什么问：不改这一环会怎样）。
    pub body: String,
    /// 目的：详情（展开看的东西：哪个目录、缺什么前提、怎么补）；没有就是空串。
    pub detail: String,
    /// 目的：选项集——`(id, label)`；**id 是行为契约**（回答按 id 分派、改文案不改行为），label 只给渲染。
    pub options: Vec<(String, String)>,
}
