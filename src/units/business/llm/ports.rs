//! 目的：llm 单元的出站端口——本单元要请人做的事，以及「供应商协议」这个可变机制的形状。
//! 管：ChatGateway / ModelCatalog / EnvelopeRepair / Protocol 四个 trait 与协议端口要用的值类型。
//! 不管：协议词汇与入站用例（在 api / service）；各协议的具体字段名（在 detail/protocol/*——**只在那里**）。
//! 联动：只由 service.rs 与 detail 持有（R12）；实现见 detail/http_chat.rs、detail/protocol/、detail/model_catalog.rs、detail/repair.rs。

use crate::units::business::llm::api::{
    BoxedChat, Channel, Chunk, Completion, Malformed, Msg, ProbeOutcome, RepairOutcome,
    ReplayReport, ToolDecl,
};
use crate::units::business::llm::domain::stream::StreamAcc;

/// 目的：供应商模型目录端口：列出一条通道当前可用的模型名（发现机制在适配层）。
pub trait ModelCatalog {
    /// 按**端点与密钥**列一条通道当前可用的模型名（不需要已选定的模型）。
    fn list_models(&self, base_url: &str, api_key: &str) -> Result<Vec<String>, String>;
}

/// 目的：通道工厂端口：网关只负责「怎么建通道」（机制）。
///    「用哪条模型通道」由登记处解析后传入（策略在协调业务）——网关不做选择；
///    没有可用通道是**调用方的前置条件失败**，不会传进来，生产网关也不得回落到假模型。
pub trait ChatGateway {
    /// 按已解析的通道建一条成员会话句柄。
    fn member_channel(&self, channel: &Channel, module_id: &str) -> BoxedChat;
    /// 按已解析的通道建核心自身通道（整理 / 验收 / 代拟 / 推荐）。
    fn core_channel(&self, channel: &Channel) -> BoxedChat;
    /// 实测这条通道支不支持原生工具调用（发两条最小请求对比：不带 tools / 带 tools）。
    /// 脚本替身没有真实供应商可测——如实返回 Err，不假装测过。
    fn probe_tools(&self, channel: &Channel) -> Result<ProbeOutcome, String>;
    /// 实测"工具调用历史怎么发回供应商才收"（回放形状）：逐项报供应商收了没有、模型真的读懂没有。
    /// 只报事实、不改登记处。默认实现如实说"测不了"——只有真正出网的网关才 override。
    fn probe_replay(&self, _channel: &Channel) -> Result<ReplayReport, String> {
        Err("这条通道没有真实供应商，测不了回放形状".to_string())
    }
}

/// 目的：信封修复端口：模型手写的工具信封不合法时，**先**问它能不能按无歧义的写法修好。
///    契约：
///    - 只做**不会产生歧义**的修补（例如把字符串里的裸控制字符转义）；改了字段含义就是错。
///    - 修不了就 repaired = None，把原因写进 what（上层据此走原路：记一条失败的工具行，让模型重发）。
///    - 宁缺毋滥：拿不准就别修——让模型重发一次，好过猜它想写什么。
///      默认真现在 `detail/repair.rs`（只做控制字符转义）；第三方实现满足本契约即可整体替换。
pub trait EnvelopeRepair: Send + Sync {
    fn repair(&self, raw: &str, kind: &Malformed) -> RepairOutcome;
}
/// 目的：一次补全要发出去的全部输入（**协议无关**）：协议实现据此拼请求体。
pub struct Request<'a> {
    /// 目的：这次要发的模型名（调用方按登记处解析好的那一个）。
    pub model: &'a str,
    /// 目的：这次要发的消息（协议无关的四种角色）。
    pub messages: &'a [Msg],
    /// 目的：要不要流式（请求体里的开关由协议翻译）。
    pub stream: bool,
    /// 目的：要声明的工具；None = 本次不声明（手写信封，或本轮不需要工具）。
    pub tools: Option<&'a [ToolDecl]>,
    /// 目的：本轮要并入请求体的思考参数片段（None = 不发）。
    pub thinking: Option<&'a serde_json::Value>,
}

/// 目的：一条流式分块处理完之后，流机制该怎么走。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStep {
    /// 继续读下一块。
    Continue,
    /// 调用方要求中止（回调返回 false）：停止读流，已产出的部分原样带回。
    Stopped,
}

/// 目的：**供应商协议**端口：wire 形状的唯一落点。
///    请求体怎么拼、回执怎么读、流式分块怎么解析、用量怎么归一——全在这里；流机制（端点、超时、
///    取消、SSE 分行、重试）与调用方都不认识任何字段名。
/// 约束：新增一种供应商协议 = 新增一个实现 + 一处选择，不改 api / 调用方 / 流机制。
pub trait Protocol: Send + Sync {
    /// 目的：思考片段不得覆盖的请求字段（与登记处同一份保守集合，合并前挡一次）。
    fn reserved_keys(&self) -> &'static [&'static str];

    /// 目的：拼一次补全的请求体（**正式会话与探针共用**：探针发出去的必须就是线上真会发的东西）。
    fn request(&self, req: &Request<'_>) -> serde_json::Value;

    /// 目的：拼一个「消息数组已经成型」的请求体（探针回放用：那些历史由 replay_histories 给出）。
    fn request_raw(
        &self,
        model: &str,
        messages: serde_json::Value,
        tools: Option<&[ToolDecl]>,
    ) -> serde_json::Value;

    /// 目的：非流式回执 → 一次补全；不是这个协议的补全形状 = Err（流机制据此换候选端点）。
    fn reply(&self, text: &str) -> Result<Completion, String>;

    /// 目的：一条流式分块（已解析成 JSON 的 data 行）→ 写进累积状态，并决定下一步。
    fn event(
        &self,
        value: &serde_json::Value,
        acc: &mut StreamAcc,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> EventStep;

    /// 目的：把「流式要 usage」的参数从请求体里摘掉；返回摘掉了没有（没有 = 这个协议没这个参数，不用重试）。
    fn drop_stream_usage(&self, body: &mut serde_json::Value) -> bool;

    /// 目的：把「上一轮调用过工具」的历史表达成本协议的几种可判定写法（回放形状探测用）。
    ///    第一项必须是**现在线上真在用的基线**；后续项是同一件事的其它写法。
    fn replay_histories(&self, nonce: &str) -> Vec<(&'static str, serde_json::Value)>;
}
