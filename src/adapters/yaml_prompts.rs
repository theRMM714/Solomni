//! 装配输入加载：`prompts/` 目录 → capabilities::prompt::api::Prompts（**只有提示词文本**），
//! `systools/` 两张表 → core::roles::SystemTools（工具是什么 + 身份有什么）。
//! 缺目录/缺文件 = 装配错误（如实报错，不静默造默认文案）。
//!
//! **两者不合并**：工具总表与角色表不是提示词；挂进册子会让提示词能力反过来依赖工具能力，
//! 两边成环（见 docs/architecture/refactor-plan.md §三）。

use crate::capabilities::prompt::api::Prompts;
use crate::capabilities::prompt::ports::PromptSource;
use crate::capabilities::tools::api::ToolBook;
use crate::capabilities::tools::api::{RoleTable, SystemTools};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 工具总表的文件形状。
#[derive(Deserialize)]
struct ToolFile {
    tools: ToolBook,
}

/// 角色表的文件形状。
#[derive(Deserialize)]
struct RoleFile {
    roles: RoleTable,
}

pub struct YamlPrompts {
    prompts: PathBuf,
    systools: PathBuf,
}

impl YamlPrompts {
    /// prompts = 提示词册目录；systools = 系统工具目录（其下 `tools.yaml` 是工具总表）。
    pub fn new(prompts: PathBuf, systools: PathBuf) -> YamlPrompts {
        YamlPrompts { prompts, systools }
    }
}

impl PromptSource for YamlPrompts {
    fn load(&self) -> Result<Prompts, String> {
        crate::capabilities::prompt::api::merge_book(&self.read_docs()?)
    }
}

impl YamlPrompts {
    /// 系统工具与角色：读 `systools/tools.yaml`（工具是什么）与 `systools/roles.yaml`（身份有什么）。
    pub fn system_tools(&self) -> Result<SystemTools, String> {
        let path = self.systools.join("tools.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("工具总表读不了（systools/tools.yaml）：{}", e))?;
        let tools: ToolFile = serde_yaml::from_str(&text)
            .map_err(|e| format!("工具总表非法（systools/tools.yaml）：{}", e))?;
        if tools.tools.is_empty() {
            return Err("工具总表里一个工具都没有（systools/tools.yaml）".to_string());
        }
        let path = self.systools.join("roles.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("角色表读不了（systools/roles.yaml）：{}", e))?;
        let roles: RoleFile = serde_yaml::from_str(&text)
            .map_err(|e| format!("角色表非法（systools/roles.yaml）：{}", e))?;
        Ok(SystemTools {
            tools: tools.tools,
            roles: roles.roles,
        })
    }
}

impl YamlPrompts {
    /// 册子的各文件（按路径排序：装配必须确定——同一份代码在任何机器上装配出同一册子）。
    fn read_docs(&self) -> Result<Vec<String>, String> {
        let mut files: Vec<PathBuf> = Vec::new();
        collect_yaml(&self.prompts, &mut files)
            .map_err(|e| format!("提示词册目录读不了（prompts/）：{}", e))?;
        if files.is_empty() {
            return Err("提示词册目录里一个 .yaml 都没有（prompts/）".to_string());
        }
        files.sort();
        let mut docs = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("提示词册文件读不了（{}）：{}", file.display(), e))?;
            docs.push(text);
        }
        Ok(docs)
    }
}

/// 递归收集 `.yaml`（不跟符号链接；子目录按路径排序后合并）。
fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_yaml(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            out.push(path);
        }
    }
    Ok(())
}
