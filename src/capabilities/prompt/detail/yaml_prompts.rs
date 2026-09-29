//! 提示词册的装配输入：`prompts/` 目录 → `Prompts`（**只有提示词文本**）。
//! 缺目录/缺文件 = 装配错误（如实报错，不静默造默认文案）。
//!
//! **工具总表与角色表不在这里**：它们是工具侧的事实，加载器在 `capabilities/tools/detail/`——
//! 挂进来会让 `prompt → tools` 成环（见 ARCHITECTURE.md §一）。

use crate::capabilities::prompt::domain::prompt::{merge_book, Prompts};
use crate::capabilities::prompt::ports::PromptSource;
use std::path::{Path, PathBuf};

pub struct YamlPrompts {
    prompts: PathBuf,
}

impl YamlPrompts {
    /// prompts = 提示词册目录。
    pub fn new(prompts: PathBuf) -> YamlPrompts {
        YamlPrompts { prompts }
    }
}

impl PromptSource for YamlPrompts {
    fn load(&self) -> Result<Prompts, String> {
        merge_book(&self.read_docs()?)
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
