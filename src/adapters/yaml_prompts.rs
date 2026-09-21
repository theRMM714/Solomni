//! 提示词册加载：`prompts/` 目录 → core::prompt::Prompts（实现 PromptSource 端口）。
//! 缺目录/缺文件 = 装配错误（如实报错，不静默造默认文案）。

use crate::core::ports::PromptSource;
use crate::core::prompt::Prompts;
use std::path::{Path, PathBuf};

pub struct YamlPrompts {
    dir: PathBuf,
}

impl YamlPrompts {
    pub fn new(dir: PathBuf) -> YamlPrompts {
        YamlPrompts { dir }
    }
}

impl PromptSource for YamlPrompts {
    fn load(&self) -> Result<Prompts, String> {
        let mut files: Vec<PathBuf> = Vec::new();
        collect_yaml(&self.dir, &mut files)
            .map_err(|e| format!("提示词册目录读不了（prompts/）：{}", e))?;
        if files.is_empty() {
            return Err("提示词册目录里一个 .yaml 都没有（prompts/）".to_string());
        }
        // 按路径排序：装配必须确定——同一份代码在任何机器上都要装配出同一册子。
        files.sort();
        let mut docs = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("提示词册文件读不了（{}）：{}", file.display(), e))?;
            docs.push(text);
        }
        crate::core::prompt::merge_book(&docs)
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
