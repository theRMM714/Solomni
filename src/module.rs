//! 模块扫描与打包契约（module.yaml）。
//! 清单即事实：扫描 modules/ 目录的纯函数，非法模块拒收并说明原因。

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// module.yaml —— 打包契约。模块对世界的全部自我介绍。
#[derive(Debug, Deserialize)]
pub struct ModuleManifest {
    pub id: String,
    pub brief: String,
    pub system: String,
    #[serde(default)]
    pub tools: Vec<String>,
    // 待接入：直连/协作建会话时读取（联动 ModelPrefs）。
    #[serde(default)]
    #[allow(dead_code)]
    pub model: ModelPrefs,
}

// 待接入：直连/协作建会话时按 prefer/provider 选模型（联动 providers.rs）。
#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct ModelPrefs {
    #[serde(default)]
    pub prefer: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

/// 一个已发现的模块 = 文件夹 + 清单。
#[derive(Debug)]
#[allow(dead_code)]
pub struct Module {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
    /// model.config.yaml 的当前选择（运行期选择记录，核心读写）。
    pub selected_provider: Option<String>,
}

impl Module {
    /// 模块 AI 的初始上下文：职责提示词 + 工具清单（工具密钥由核心拉起时注入，不在此展开）。
    pub fn system_block(&self) -> String {
        let mut s = self.manifest.system.trim().to_string();
        if !self.manifest.tools.is_empty() {
            s.push_str("\n\n本模块可用工具：");
            for t in &self.manifest.tools {
                s.push_str(&format!("\n- {}", t));
            }
        }
        s
    }
}

/// 扫描结果：合法模块 + 拒收原因（校验，不是挑选——如实呈现）。
pub struct Roster {
    pub modules: Vec<Module>,
    pub rejected: Vec<String>,
}

/// 列出 modules/ 下每个含合法 module.yaml 的文件夹。
/// 放入即出现，移出即消失；id 与文件夹名不一致 = 非法，拒收并说明原因。
pub fn scan(modules_dir: &Path) -> Roster {
    let mut modules = Vec::new();
    let mut rejected = Vec::new();
    let entries = match std::fs::read_dir(modules_dir) {
        Ok(e) => e,
        Err(_) => return Roster { modules, rejected },
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let yaml_path = path.join("module.yaml");
        let yaml_text = match std::fs::read_to_string(&yaml_path) {
            Ok(t) => t,
            Err(_) => {
                rejected.push(format!(
                    "{}: 缺少 module.yaml",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                continue;
            }
        };
        match serde_yaml::from_str::<ModuleManifest>(&yaml_text) {
            Ok(m) => {
                let dir_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if m.id != dir_name {
                    rejected.push(format!(
                        "{}: id '{}' 与文件夹名不一致",
                        dir_name, m.id
                    ));
                    continue;
                }
                let selected_provider =
                    std::fs::read_to_string(path.join("model.config.yaml"))
                        .ok()
                        .and_then(|t| serde_yaml::from_str::<ProviderRef>(&t).ok())
                        .and_then(|r| r.provider);
                modules.push(Module { manifest: m, root: path, selected_provider });
            }
            Err(e) => rejected.push(format!(
                "{}: module.yaml 非法（{}）",
                path.file_name().unwrap_or_default().to_string_lossy(),
                e
            )),
        }
    }
    modules.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    Roster { modules, rejected }
}

/// model.config.yaml —— 运行期供应商选择记录（只有 id，永不密钥）。
#[derive(Debug, Deserialize)]
pub struct ProviderRef {
    pub provider: Option<String>,
}
