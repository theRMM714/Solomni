//! 本能力的**用例与端口持有者**：三个出站端口只在这里（R12）——
//! 通道工厂 / 模型目录 / 信封修复。别的能力要通道、要探测、要发现，走 `api::Llm`。
//!
//! 装配（new 出适配器）在组合根；这里只收注入的端口。

use crate::capabilities::llm::api::{
    BoxedChat, Channel, Llm, Malformed, ProbeOutcome, RepairOutcome, ReplayReport,
};
use crate::capabilities::llm::ports::{ChatGateway, EnvelopeRepair, ModelCatalog};
use std::sync::Arc;

/// 模型通道能力：持端口，按用例答话。
pub struct LlmService {
    gateway: Arc<dyn ChatGateway + Send + Sync>,
    catalog: Arc<dyn ModelCatalog + Send + Sync>,
    repair: Arc<dyn EnvelopeRepair + Send + Sync>,
}

impl LlmService {
    /// 组合根专用。
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
    fn member_channel(
        &self,
        channel: Option<&Channel>,
        module_id: &str,
    ) -> (BoxedChat, Option<String>) {
        self.gateway.member_channel(channel, module_id)
    }

    fn core_channel(&self, channel: Option<&Channel>) -> (BoxedChat, bool) {
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
