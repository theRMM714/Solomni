//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。
//! 这里的名字就是本能力对外的契约；各文案结构体与渲染实现留在 `domain/`。

pub use crate::capabilities::prompt::domain::prompt::{
    merge_book, Prompts, RefsPrompts, ToolTexts,
};
pub use crate::capabilities::prompt::domain::refs::{rewrite, RefRoots};
