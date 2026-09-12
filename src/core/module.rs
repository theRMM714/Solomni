//! 模块打包契约（module.yaml）与扫描结果。
//! 目录遍历机制在 adapters（ModuleSource 端口）；「清单即事实」的重扫策略由 core 执行。

use serde::Deserialize;
use std::path::PathBuf;

/// module.yaml —— 打包契约。模块对世界的全部自我介绍。
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleManifest {
    pub id: String,
    pub brief: String,
    pub system: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub model: ModelPrefs,
}

/// 模型偏好：按 prefer/provider 选模型（联动 providers 解析链）。
// 预留字段：prefer 供多模型供应商下按用途选模型（实现期接入）。
#[derive(Debug, Clone, Default, Deserialize)]
#[allow(dead_code)]
pub struct ModelPrefs {
    #[serde(default)]
    pub prefer: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

/// 一个已发现的模块 = 文件夹 + 清单。
// 预留字段：root 供工具执行器定位模块工作区（工具接入期使用）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Module {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
    /// model.config.yaml 的当前选择（运行期选择记录，只有 id，永不密钥）。
    pub selected_provider: Option<String>,
}

impl Module {
    /// 模块 AI 的初始上下文：职责提示词 + 工具清单。
    pub fn system_block(&self, prompts: &crate::core::prompt::Prompts) -> String {
        if self.manifest.tools.is_empty() {
            prompts.render(&prompts.core.module_system, &[("system", self.manifest.system.trim().to_string())])
        } else {
            let tools = self
                .manifest
                .tools
                .iter()
                .map(|t| format!("- {}", t))
                .collect::<Vec<_>>()
                .join("\n");
            prompts.render(
                &prompts.core.module_system_tools,
                &[("system", self.manifest.system.trim().to_string()), ("tools", tools)],
            )
        }
    }
}

/// 扫描结果：合法模块 + 拒收原因（校验，不是挑选——如实呈现）。
pub struct Roster {
    pub modules: Vec<Module>,
    pub rejected: Vec<String>,
}

/// model.config.yaml —— 运行期供应商选择记录（只有 id，永不密钥）。
#[derive(Debug, Deserialize)]
pub struct ProviderRef {
    pub provider: Option<String>,
}
