//! 目的：llm 能力的用例与端口持有者。
//! 管：实现 api::Llm（造通道 / 探测 / 发现 / 修信封）；持有三个出站端口。
//! 不管：适配器装配（在组合根）；协议类型定义（在 api）；域内纯逻辑（在 domain）。
//! 联动：组合根 new 时注入适配器；其它能力与 conductor 经 api::Llm 消费（R12）。

use crate::capabilities::llm::api::{
    BoxedChat, Channel, Llm, Malformed, ProbeOutcome, RepairOutcome, ReplayReport,
};
use crate::capabilities::llm::ports::{ChatGateway, EnvelopeRepair, ModelCatalog};
use std::sync::Arc;

/// 目的：模型通道能力：持端口，按用例答话。
pub struct LlmService {
    gateway: Arc<dyn ChatGateway + Send + Sync>,
    catalog: Arc<dyn ModelCatalog + Send + Sync>,
    repair: Arc<dyn EnvelopeRepair + Send + Sync>,
}

impl LlmService {
    /// 目的：组合根专用。
    pub fn new(
        gateway: Arc<dyn ChatGateway + Send + Sync>,
        catalog: Arc<dyn ModelCatalog + Send + Sync>,
        repair: Arc<dyn EnvelopeRepair + Send + Sync>,
    ) -> LlmService {
        LlmService {
            gateway,
            catalog,
            repair,
        }
    }
}

impl Llm for LlmService {
    fn member_channel(&self, channel: &Channel, module_id: &str) -> BoxedChat {
        self.gateway.member_channel(channel, module_id)
    }

    fn core_channel(&self, channel: &Channel) -> BoxedChat {
        self.gateway.core_channel(channel)
    }

    fn probe_tools(&self, channel: &Channel) -> Result<ProbeOutcome, String> {
        self.gateway.probe_tools(channel)
    }

    fn probe_replay(&self, channel: &Channel) -> Result<ReplayReport, String> {
        self.gateway.probe_replay(channel)
    }

    fn list_models(&self, base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
        self.catalog.list_models(base_url, api_key)
    }

    fn repair(&self, raw: &str, kind: &Malformed) -> RepairOutcome {
        self.repair.repair(raw, kind)
    }
}
