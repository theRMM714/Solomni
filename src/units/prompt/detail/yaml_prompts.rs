//! 目的：提示词册的装配输入——`prompts/` 目录加载成 Prompts（只有提示词文本）。
//! 管：实现 ports::PromptSource；经 `Assets` 收文档；缺目录 / 缺文件如实报装配错误。
//! 不管：目录枚举与读盘（在 `assets`）；工具总表与机制席位（在 units/tools/detail）；渲染与 `@` 引用改写（在 domain）。
//! 联动：实现 prompt/ports.rs 的 PromptSource，由组合根装配；加载结果交 service::load。

use crate::units::prompt::domain::prompt::{Prompts, merge_book};
use crate::units::prompt::ports::PromptSource;
use crate::units::resource::assets::api::Assets;
use std::sync::Arc;

pub struct YamlPrompts {
    assets: Arc<dyn Assets>,
}

impl YamlPrompts {
    /// 目的：`assets` = 随包只读内容的入站面（`prompts/` 的落点、枚举与排序都归它）。
    pub fn new(assets: Arc<dyn Assets>) -> YamlPrompts {
        YamlPrompts { assets }
    }
}

impl PromptSource for YamlPrompts {
    fn load(&self) -> Result<Prompts, String> {
        let docs = self.assets.prompt_docs()?;
        let texts: Vec<String> = docs.into_iter().map(|d| d.text).collect();
        merge_book(&texts)
    }
}
