//! 供应商登记处（内存结构与解析链）。
//! 持久化机制在 adapters（ProviderStore 端口）；密钥只存在于登记处数据与出站调用。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 登记处（providers.yaml 的内存形态）。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub kind: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<String>,
}

impl Registry {
    /// 解析链：模块当前选择 > 清单默认 > 全局默认。
    /// 用户显式选择（module_choice）不存在 = None——不静默改用别的供应商（不猜测原则，
    /// 交由网关回落演示通道并如实告知）；未显式选择才顺链回落。
    pub fn resolve(
        &self,
        module_choice: Option<&str>,
        manifest_default: Option<&str>,
    ) -> Option<(String, &Provider)> {
        if let Some(k) = module_choice {
            return self.providers.get(k).map(|p| (k.to_string(), p));
        }
        for key in [manifest_default, self.default.as_deref()] {
            if let Some(k) = key {
                if let Some(p) = self.providers.get(k) {
                    return Some((k.to_string(), p));
                }
            }
        }
        None
    }

    /// 展示用行：永不包含密钥。
    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (id, p) in &self.providers {
            let mark = if self.default.as_deref() == Some(id.as_str()) { "（默认）" } else { "" };
            let models = if p.models.is_empty() { "—".to_string() } else { p.models.join(",") };
            lines.push(format!("{}{}  {}  模型：{}", id, mark, p.base_url, models));
        }
        lines
    }
}