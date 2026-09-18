//! 供应商与模型登记处（内存形态）与通道解析。
//! 供应商 = 端点 + 密钥；模型 = 独立实体（展示名 + 实际模型串 + 所属供应商 + 能力说明）。
//! 两者分开保存（机制在适配层的两个文件）；密钥只存在于登记处与核心发起的出站调用。
//! 「模型 → 供应商」的绑定只对本产品可见；模块与会话只持模型 id 引用。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一条供应商通道（端点 + 密钥）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub base_url: String,
    pub api_key: String,
}

/// 一条可用模型：展示名 + 实际模型串 + 所属供应商 + 能力说明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub api_model: String,
    pub provider: String,
    #[serde(default)]
    pub note: String,
}

/// 基本设置（settings.yaml）：一般 agent 都有的开关。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// 流式传输：向供应商请求逐片返回。
    #[serde(default = "default_true")]
    pub streaming: bool,
    /// 思维链显示：开启后每条回答下的思维链块才出现（永远默认折叠，点击展开）。
    #[serde(default = "default_true")]
    pub show_reasoning: bool,
    /// 默认执行档位：新建会话未单独选定时用它（本机 = 在宿主上跑；虚拟机 = 整台 guest）。
    #[serde(default)]
    pub tier: crate::core::exec::Tier,
    /// 是否允许工具围栏在本机写权限（Windows 上要给会话/模块目录与解释器安装目录加目录 ACL）。
    /// 默认否：没经过用户显式授权，本程序不动本机任何权限项。
    #[serde(default)]
    pub fence_write: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings { streaming: true, show_reasoning: true, tier: crate::core::exec::Tier::Host, fence_write: false }
    }
}

/// 登记处（providers.yaml + models.yaml + settings.yaml + agents.yaml 的内存合体）。
#[derive(Debug, Default, Clone)]
pub struct Settings {
    pub providers: BTreeMap<String, Provider>,
    pub models: BTreeMap<String, ModelEntry>,
    /// 核心 AI 默认模型 id。
    pub core: Option<String>,
    /// 基本设置。
    pub app: AppSettings,
    /// 用户配置的具名 agent。
    pub agents: crate::core::agents::Agents,
}

/// 成品通道：core 解析后交适配层建会话；适配层不再做任何选择。
#[derive(Debug, Clone)]
pub struct Channel {
    pub provider: Provider,
    pub model: String,
}

impl Settings {
    /// 模型 id → 成品通道；模型或供应商缺失 = 报错（不猜测回退）。
    pub fn resolve(&self, model_id: &str) -> Result<Channel, String> {
        let m = self.models.get(model_id).ok_or_else(|| format!("无此模型：{}", model_id))?;
        let p = self
            .providers
            .get(&m.provider)
            .ok_or_else(|| format!("模型 {} 引用的供应商不存在：{}", model_id, m.provider))?;
        Ok(Channel { provider: p.clone(), model: m.api_model.clone() })
    }

    /// 核心 AI 默认通道；未设定或解析失败 = None（调用方回落演示并如实告知）。
    pub fn core_channel(&self) -> Option<Channel> {
        self.core.as_deref().and_then(|id| self.resolve(id).ok())
    }

    /// 供应商展示视图（永不携带密钥）。
    pub fn provider_views(&self) -> Vec<ProviderView> {
        self.providers
            .iter()
            .map(|(id, p)| ProviderView { id: id.clone(), base_url: p.base_url.clone() })
            .collect()
    }

    /// 模型展示视图（含「是否核心默认」）。
    pub fn model_views(&self) -> Vec<ModelView> {
        self.models
            .iter()
            .map(|(id, m)| ModelView {
                id: id.clone(),
                name: m.name.clone(),
                api_model: m.api_model.clone(),
                provider: m.provider.clone(),
                note: m.note.clone(),
                is_core: self.core.as_deref() == Some(id.as_str()),
            })
            .collect()
    }

}

/// 供应商展示视图（不含密钥）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderView {
    pub id: String,
    pub base_url: String,
}

/// 模型展示视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub api_model: String,
    pub provider: String,
    pub note: String,
    pub is_core: bool,
}
