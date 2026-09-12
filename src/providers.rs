//! 供应商登记处：产品级资源，密钥唯一合法居所（核心私有区）。
//! 模块与工具只持引用（id）；通道只对核心的出站调用开放。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// 登记处文件（providers.yaml）：用户经产品命令管理。
#[derive(Debug, Default, Serialize, Deserialize)]
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
    /// 从核心私有区加载；不存在 = 空登记处（产品照常可跑假模型）。
    pub fn load(path: &Path) -> Registry {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_yaml::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// 路由一个模块的供应商 id：模块当前选择 > 清单默认 > 产品全局默认。
    /// 返回 None = 没有任何可用通道（如实告知，不静默造）。
// 待接入：供应商路由：协作/直连模式按此选通道（联动 module.rs 的 model.config.yaml 读取）
    #[allow(dead_code)]
    pub fn resolve(&self, module_choice: Option<&str>, manifest_default: Option<&str>) -> Option<(String, &Provider)> {
        for cand in [module_choice, manifest_default, self.default.as_deref()] {
            if let Some(id) = cand {
                if let Some(p) = self.providers.get(id) {
                    return Some((id.to_string(), p));
                }
            }
        }
        None
    }
}
