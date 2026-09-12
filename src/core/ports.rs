//! 核心端口：依赖倒置的边界。core 定义，adapters 实现，main 注入。
//! 端口语义：
//! - Chat：一次模型会话（收消息列表，回原始文本）。
//! - ProviderStore：供应商登记处持久化（核心只认内存结构；yaml/0600 机制在适配层）。
//! - ModuleSource：模块清单来源（「清单即事实」的重扫策略由 core 执行；目录遍历机制在适配层）。
//! - ChatGateway：通道工厂（真实通道/演示回落的选择在适配层；回落必须如实告知）。
//! - PromptSource：提示词册加载（文件机制在适配层；渲染纯逻辑在 core/prompt.rs）。

use crate::core::module::Roster;
use crate::core::providers::{Provider, Registry};
use crate::core::prompt::Prompts;

/// 一次模型会话：收消息列表，回原始文本。
pub trait Chat {
    fn complete(&mut self, messages: &[Msg]) -> Raw;
}

/// 拥有所有权的会话通道（装箱端口对象；会话可跨线程移动，Web 泵线程所需）。
pub type BoxedChat = Box<dyn Chat + Send>;

/// 一条消息：role = system / user / assistant。
#[derive(Debug, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
}

impl Msg {
    pub fn system(content: impl Into<String>) -> Msg { Msg { role: "system".into(), content: content.into() } }
    pub fn user(content: impl Into<String>) -> Msg { Msg { role: "user".into(), content: content.into() } }
    pub fn assistant(content: impl Into<String>) -> Msg { Msg { role: "assistant".into(), content: content.into() } }
}

/// 一次补全的原始文本输出。
pub type Raw = String;

/// 登记处持久化端口。
pub trait ProviderStore {
    fn load(&self) -> Result<Registry, String>;
    fn save(&self, registry: &Registry) -> Result<(), String>;
}

/// 模块清单来源端口。
pub trait ModuleSource {
    fn scan(&self) -> Roster;
}

/// 通道工厂端口：网关只负责「怎么建通道」（机制）。
/// 「用哪个供应商」由 core 解析链决定后传入（策略在 core）——网关不做选择。
/// provider = None 表示无可用供应商：实现方必须回落演示通道并如实告知（不得静默）。
pub trait ChatGateway {
    fn member_channel(&self, provider: Option<&Provider>, module_id: &str) -> (BoxedChat, Option<String>);
    /// 核心自身通道（整理/验收/代拟）；bool = 是否演示通道（供如实告知）。
    fn core_channel(&self, provider: Option<&Provider>) -> (BoxedChat, bool);
}

/// 提示词册加载端口。
pub trait PromptSource {
    fn load(&self) -> Result<Prompts, String>;
}

/// 运行日志端口：关键节点（异常/降级/边界）落盘，供事后确定问题，避免过度推理。
/// core 只调用；文件/时间戳/目录机制在适配层。
pub trait Log: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

/// 测试与纯逻辑场景的无声日志（不落任何盘）。
pub struct NoopLog;
impl Log for NoopLog {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}