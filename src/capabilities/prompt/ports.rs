//! 提示词册的**出站端口**：册子从哪加载（机制在适配层）。

use crate::capabilities::prompt::domain::prompt::Prompts;

/// 提示词册加载端口。
pub trait PromptSource {
    fn load(&self) -> Result<Prompts, String>;
}
