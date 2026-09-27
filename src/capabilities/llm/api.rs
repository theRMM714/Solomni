//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。
//!
//! 两样东西在这里：
//! - **通道与协议词汇**（`Chat` / `BoxedChat` / `Msg` / `Completion` / `Chunk` / `ToolCall` /
//!   `ToolDecl` / `Channel` / `ToolMode` / `LlmOpts` / `CompleteOpts` / 探测结论…）——
//!   它们是**对外的契约**：会话拿着通道句柄说话，别人读通道事实。
//!   `Chat` 之所以在 api 而不在 ports：**它的调用方是别的能力**；ports 是本能力自己出站用的抽象（R12）。
//! - **`Llm` 用例面**（`service.rs` 实现）：造通道 / 探测 / 发现 / 修信封；
//!   出站端口（`ChatGateway` / `ModelCatalog` / `EnvelopeRepair`）**只由它持有**，不出现在 api 里。

pub use crate::capabilities::llm::domain::envelope::{
    parse, Malformed, Reply, Tail, ToolInvoke, Verb,
};
pub use crate::capabilities::llm::domain::malformed::malformed_report;

use crate::kernel::types::DEFAULT_LLM_TIMEOUT_SECS;
use serde::{Deserialize, Serialize};

/// 一种"回放形状"的探测结论：**收了没有**（HTTP 层）+ **看懂了没有**（回答里带回了工具结果里的编号）
/// + 供应商原话或回答片段。事实，不是猜测。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReplayShape {
    pub name: String,
    pub accepted: bool,
    pub understood: bool,
    pub detail: String,
}

/// 回放形状探测报告：形状按探测顺序排列，第一项是基线（现在线上真在用的形状）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReplayReport {
    pub shapes: Vec<ReplayShape>,
}

/// 该通道的工具调用形态（登记处里的事实；**缺省 envelope** = 任何供应商都能用的手写信封）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    /// 手写信封：模型在正文里写 {"type":"tool",…}，核心解析。任何供应商都支持。
    #[default]
    Envelope,
    /// 原生工具调用：参数走供应商的结构化槽位（需要该通道确实支持 function calling）。
    Native,
}

/// 成品通道：登记处**解析后的结果**（端点 + 密钥 + 实际模型串），交给通道层建会话；
/// 适配层不再做任何选择。**刻意摊平**（不嵌 `registry::Provider`）：嵌回去会让 `llm → registry` 成环。
#[derive(Debug, Clone)]
pub struct Channel {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// 流式片段：一次调用的起点 / 正文 / 思维链。
#[derive(Debug, Clone)]
pub enum Chunk {
    /// 每次模型调用的起点：调用方据此重置本轮累积（工具多轮不会糊在一起）。
    Start,
    Text(String),
    Reasoning(String),
}

/// 声明给供应商的一个工具（原生工具调用通道用）：名字 + 说明 + JSON Schema 参数。
/// 说明与 Schema 都来自文本层（内置工具在 prompts/、模块工具在 module.yaml），这里只是搬运形态。
#[derive(Debug, Clone)]
pub struct ToolDecl {
    pub name: String,
    pub description: String,
    /// JSON Schema（由 schema 的声明渲染；模块没声明参数时是"不收参数"的空对象结构）。
    pub parameters: serde_json::Value,
}

/// 一次模型调用的通道参数：**策略在 core 定**（都来自全局设置），机制在适配器。
/// 一个值一路传下去，而不是把 stream / 预算分别塞进各个函数的参数表——两处各传一份迟早会漏。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlmOpts {
    /// 要不要流式（全局设置 `streaming`）：讨论、执行、验收、单 agent 全用它。
    pub stream: bool,
    /// 单次调用总预算（全局设置 `llm_timeout_secs`）。
    pub timeout_secs: u64,
}

impl Default for LlmOpts {
    fn default() -> Self {
        LlmOpts {
            stream: false,
            timeout_secs: DEFAULT_LLM_TIMEOUT_SECS,
        }
    }
}

/// 一次补全的请求选项：策略在 core（要不要流式、要不要声明工具、给多少预算），机制在适配器。
/// Copy：讨论回合的工具循环每轮都要一份（只换 tools 槽位，其余照旧）。
#[derive(Clone, Copy)]
pub struct CompleteOpts<'a> {
    pub stream: bool,
    /// 要声明的工具；None = 本次不声明（手写信封模式，或本轮不需要工具）。
    pub tools: Option<&'a [ToolDecl]>,
    /// 本次调用的**总预算**（秒）：连接之外，等响应头、读响应体与整体都用它。
    /// 为什么是一个预算而不是拆几个：**非流式**下供应商要等整段生成完才发响应头，
    /// 单独设一个小的"头超时"会把长回复误判成不通（真机上就是这么炸的：49~53 秒的回复撞了 60 秒头超时）。
    pub timeout_secs: u64,
}

impl<'a> CompleteOpts<'a> {
    /// 本次不声明工具（手写信封模式；测试替身与演示通道也用它）。
    pub fn plain(stream: bool) -> CompleteOpts<'a> {
        CompleteOpts {
            stream,
            tools: None,
            timeout_secs: DEFAULT_LLM_TIMEOUT_SECS,
        }
    }

    /// 带上本次预算（核心按设置给；设置是全局的，见 `AppSettings::llm_timeout_secs`）。
    pub fn with_timeout(mut self, secs: u64) -> CompleteOpts<'a> {
        self.timeout_secs = secs;
        self
    }
}

/// 供应商返回的一次**原生**工具调用。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// 供应商给的调用 id：回结果时必须原样带上（OpenAI 协议的 tool_call_id）。
    pub id: String,
    pub name: String,
    /// 参数原样（供应商给的是一段 JSON 文本；能不能解析由上层如实报，不在这里猜）。
    pub args_json: String,
}

/// 一次模型会话：收消息列表，回正文 + 供应商给的**结束原因**。
/// stream = 要求供应商流式返回；on 逐片回调（非流式实现不回调）。
/// on 返回 false = 调用方要求中止，实现方必须立即停止读取并返回已产出的正文。
pub trait Chat {
    fn complete(
        &mut self,
        messages: &[Msg],
        opts: CompleteOpts<'_>,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion;
}

/// 拥有所有权的会话通道（装箱；会话可跨线程移动，Web 泵线程所需）。
pub type BoxedChat = Box<dyn Chat + Send>;

/// 一条消息：role = system / user / assistant / tool。
///
/// 后两个字段是**原生工具调用**的协议字段（手写信封通道恒为空，也就不进 wire）：
/// 一次回复可以有**多个**调用，所以调用是数组挂在助手消息上；结果消息靠 tool_call_id 回应它们。
/// 回放（重启/回档后重建上下文）与实时必须产出同样的消息——唯一的构造函数见 engine::reply_msgs。
#[derive(Debug, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
    /// 这条助手消息发起了哪些调用（空 = 不发这个字段）。
    pub tool_calls: Vec<ToolCall>,
    /// role = tool 时它回应的是哪个调用 id（其余角色为空）。
    pub tool_call_id: String,
}

impl Msg {
    pub fn system(content: impl Into<String>) -> Msg {
        Msg::plain("system", content)
    }
    pub fn user(content: impl Into<String>) -> Msg {
        Msg::plain("user", content)
    }
    pub fn assistant(content: impl Into<String>) -> Msg {
        Msg::plain("assistant", content)
    }
    /// 一次回复的助手消息：正文 + 它发起的**全部**调用（一次回复多个调用就靠它）。
    pub fn assistant_calls(content: impl Into<String>, calls: Vec<ToolCall>) -> Msg {
        Msg {
            tool_calls: calls,
            ..Msg::plain("assistant", content)
        }
    }
    /// 一条工具结果：回应某个调用 id（协议要求与助手消息里的调用成对出现）。
    pub fn tool(call_id: &str, content: impl Into<String>) -> Msg {
        Msg {
            tool_call_id: call_id.to_string(),
            ..Msg::plain("tool", content)
        }
    }
    fn plain(role: &str, content: impl Into<String>) -> Msg {
        Msg {
            role: role.to_string(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
        }
    }
}

/// 一次补全的结果：正文 + 供应商给的**结束原因**（原样带回，不翻译）。
/// 为什么必须有它：只有把"模型写完自己停了"与"被输出长度截断"分开，核心才能给对修法。
/// 真实会话里模型手写了一个 3KB 的工具信封、尾巴少一个括号，我们看不出是被截断还是它自己写漏，
/// 只能笼统让它重发——它照着"内容过长"的假设去分两次写，白跑两轮。
#[derive(Debug, Clone)]
pub struct Completion {
    /// 供应商返回的正文（原样，不做任何修补）。
    pub raw: String,
    /// 供应商返回的思维链（非流式响应也可提供）。
    pub reasoning: String,
    /// 结束原因原样（供应商没给 = 空串）：stop / length / content_filter / tool_calls …
    pub finish: String,
    /// 原生工具调用（按供应商给的顺序；手写信封模式恒为空）。
    pub calls: Vec<ToolCall>,
    /// 这次调用**失败**了（超时 / 网络 / 形状不对）：非空 = 没有拿到模型的回复。
    /// 为什么必须与正文分开：失败原因若当成正文，会作为**模型发言**落进转录，
    /// 而核心正是按转录派生"下一步该谁说话"——错误文本一旦混进去，状态就歪了。
    /// 有它，上层才能如实告知用户并**中断**这一轮（而不是假装模型说了这句话）。
    pub error: Option<String>,
}

impl Completion {
    /// 没有结束原因、没有原生调用的通道（演示通道、测试替身）：只有正文。
    pub fn text(raw: impl Into<String>) -> Completion {
        Completion {
            raw: raw.into(),
            reasoning: String::new(),
            finish: String::new(),
            calls: Vec::new(),
            error: None,
        }
    }

    /// 这次调用没成功：只带原因，**不带正文**（上层据此如实告知并中断）。
    pub fn failure(reason: impl Into<String>) -> Completion {
        Completion {
            raw: String::new(),
            reasoning: String::new(),
            finish: String::new(),
            calls: Vec::new(),
            error: Some(reason.into()),
        }
    }

    /// 这一次是不是被输出长度截断的。
    pub fn truncated(&self) -> bool {
        truncated(&self.finish)
    }
}

/// 结束原因算不算"被截断"：各家取值都归到这里，判定只写一次。
pub fn truncated(finish: &str) -> bool {
    matches!(finish, "length" | "max_tokens" | "max_output_tokens")
}

/// 探测结论：这条通道到底支不支持原生工具调用（**事实**，不是猜测；三种都如实回报）。
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    /// 支持：供应商真的返回了工具调用。
    Supported { detail: String },
    /// 明确不支持：不带 tools 的请求能通，带上 tools 的请求被供应商拒（附供应商原话）。
    Unsupported { detail: String },
    /// 无法判定：供应商接受了 tools 参数，但这次没有发起调用（可能只是模型没选它）。
    Unknown { detail: String },
}

/// 一次信封修复的结果。
pub struct RepairOutcome {
    /// 修好后的全文；None = 没修（核心按"不合法"处理，让它重发）。
    pub repaired: Option<String>,
    /// 如实记录做了什么（拼进工具行回执：模型与用户都看得到核心没有瞎猜）。
    pub what: Vec<String>,
}

/// 本能力对外的**用例面**（`service.rs` 实现）：别的能力要通道、要探测、要发现时走这里，
/// **不持本能力的出站端口**（R12）。
pub trait Llm: Send + Sync {
    /// 造一条成员通道句柄（`module_id` 用于演示降级时的名号）；附带是否演示回落（如实告知）。
    fn member_channel(
        &self,
        channel: Option<&Channel>,
        module_id: &str,
    ) -> (BoxedChat, Option<String>);

    /// 造核心自身通道（整理 / 验收 / 代拟 / 推荐）；bool = 是否演示通道。
    fn core_channel(&self, channel: Option<&Channel>) -> (BoxedChat, bool);

    /// 实测这条通道支不支持原生工具调用（发两条最小请求对比）。
    fn probe_tools(&self, channel: &Channel) -> Result<ProbeOutcome, String>;

    /// 实测"工具调用历史怎么发回供应商才收"（只报事实，不改登记处）。
    fn probe_replay(&self, channel: &Channel) -> Result<ReplayReport, String>;

    /// 按端点与密钥列出一条通道当前可用的模型名（发现机制在适配层）。
    fn list_models(&self, base_url: &str, api_key: &str) -> Result<Vec<String>, String>;

    /// 手写的工具信封不合法时，**先**问它能不能按无歧义的写法修好。
    fn repair(&self, raw: &str, kind: &Malformed) -> RepairOutcome;
}
