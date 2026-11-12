//! 目的：提示词册的装配输入——prompts/ 目录加载成 Prompts（只有提示词文本）。
//! 管：实现 ports::PromptSource；读目录与 yaml；缺目录 / 缺文件如实报装配错误。
//! 不管：工具总表与角色表（在 capabilities/tools/detail）；渲染与 `@` 引用改写（在 domain）。
//! 联动：实现 prompt/ports.rs 的 PromptSource，由组合根装配；加载结果交 service::load。

use crate::capabilities::prompt::domain::prompt::{Prompts, merge_book};
use crate::capabilities::prompt::ports::PromptSource;
use std::path::{Path, PathBuf};

pub struct YamlPrompts {
    prompts: PathBuf,
}

impl YamlPrompts {
    /// 目的：prompts = 提示词册目录。
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
