//! 本能力的**出站端口**：只有 `service.rs` 持有（R12——别的能力一律不许引 `ports`）：
//! 通道工厂（`ChatGateway`）、模型目录（`ModelCatalog`）、信封修复（`EnvelopeRepair`）。
//!
//! 协议词汇（`Chat` / `Msg` / `Completion` / `Channel` …）在 `api`：它们是对外契约，
//! 而这里只放"本能力要请人做的事"。策略（用哪条通道、要不要流式、声明哪些工具）在 core 定；机制在适配层。

use crate::capabilities::llm::api::{
    BoxedChat, Channel, Malformed, ProbeOutcome, RepairOutcome, ReplayReport,
};

/// 供应商模型目录端口：列出一条通道当前可用的模型名（发现机制在适配层）。
pub trait ModelCatalog {
    /// 按**端点与密钥**列一条通道当前可用的模型名（不需要已选定的模型）。
    fn list_models(&self, base_url: &str, api_key: &str) -> Result<Vec<String>, String>;
}

/// 通道工厂端口：网关只负责「怎么建通道」（机制）。
/// 「用哪条模型通道」由登记处解析后传入（策略在协调业务）——网关不做选择。
/// channel = None 表示无可用模型：实现方必须回落演示通道并如实告知（不得静默）。
pub trait ChatGateway {
    fn member_channel(
        &self,
        channel: Option<&Channel>,
        module_id: &str,
    ) -> (BoxedChat, Option<String>);
    /// 核心自身通道（整理/验收/代拟/推荐）；bool = 是否演示通道（供如实告知）。
    fn core_channel(&self, channel: Option<&Channel>) -> (BoxedChat, bool);
    /// 实测这条通道支不支持原生工具调用（发两条最小请求对比：不带 tools / 带 tools）。
    /// 演示与脚本替身没有真实供应商可测——如实返回 Unknown，不假装测过。
    fn probe_tools(&self, channel: &Channel) -> Result<ProbeOutcome, String>;
    /// 实测"工具调用历史怎么发回供应商才收"（回放形状）：逐项报供应商收了没有、模型真的读懂没有。
    /// 只报事实、不改登记处。默认实现如实说"测不了"——只有真正出网的网关才 override。
    fn probe_replay(&self, _channel: &Channel) -> Result<ReplayReport, String> {
        Err("这条通道没有真实供应商，测不了回放形状".to_string())
    }
}

/// 信封修复端口：模型手写的工具信封不合法时，**先**问它能不能按无歧义的写法修好。
/// 契约：
/// - 只做**不会产生歧义**的修补（例如把字符串里的裸控制字符转义）；改了字段含义就是错。
/// - 修不了就 repaired = None，把原因写进 what（上层据此走原路：记一条失败的工具行，让模型重发）。
/// - 宁缺毋滥：拿不准就别修——让模型重发一次，好过猜它想写什么。
///
/// 默认真现在 `detail/repair.rs`（只做控制字符转义）；第三方实现满足本契约即可整体替换。
pub trait EnvelopeRepair: Send + Sync {
    fn repair(&self, raw: &str, kind: &Malformed) -> RepairOutcome;
}
