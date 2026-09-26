//! 装配输入加载：`prompts/` 目录 → core::prompt::Prompts，`systools/tools.yaml` → 内置工具声明。
//! 缺目录/缺文件 = 装配错误（如实报错，不静默造默认文案）。
//!
//! 为什么工具声明单独一个文件：它是**工具总表**的内容（工具是什么），不是提示词。
//! 但内存形态仍挂在册子上（`Prompts.core.builtin_tools`）——消费点因此不用改，
//! 只是"这一份声明的家"从册子搬到了总表。

use crate::core::ports::PromptSource;
use crate::core::prompt::Prompts;
use crate::core::roles::{RoleTable, SystemTools};
use crate::core::schema::ToolBook;
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
        let mut book = crate::core::prompt::merge_book(&self.read_docs()?)?;
        // 工具声明与角色来自**两张表**（唯一真相）：册子里不再有这些。
        let st = self.system_tools()?;
        book.core.builtin_tools = st.tools.clone();
        book.systools = st;
        Ok(book)
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
