//! 目的：prompt 能力的出站端口——册子从哪加载。
//! 管：PromptSource trait 的定义。
//! 不管：具体文件机制（在 detail/yaml_prompts.rs）；渲染（在 domain）。
//! 联动：只由 service.rs 使用；实现见 detail/yaml_prompts.rs。

use crate::capabilities::prompt::domain::prompt::Prompts;

/// 目的：提示词册加载端口。
pub trait PromptSource {
    fn load(&self) -> Result<Prompts, String>;
}
