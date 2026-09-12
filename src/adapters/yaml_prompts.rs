//! 提示词册加载：prompts.yaml → core::prompt::Prompts（实现 PromptSource 端口）。
//! 缺文件 = 装配错误（如实报错，不静默造默认文案）。

use crate::core::ports::PromptSource;
use crate::core::prompt::Prompts;
use std::path::PathBuf;

pub struct YamlPrompts {
    path: PathBuf,
}

impl YamlPrompts {
    pub fn new(path: PathBuf) -> YamlPrompts {
        YamlPrompts { path }
    }
}

impl PromptSource for YamlPrompts {
    fn load(&self) -> Result<Prompts, String> {
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("提示词册缺失（prompts.yaml）：{}", e))?;
        serde_yaml::from_str(&text).map_err(|e| format!("prompts.yaml 非法：{}", e))
    }
}
