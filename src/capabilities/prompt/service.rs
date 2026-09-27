//! 提示词册的**能力面实现与装配入口**。
//!
//! 状态就是 `domain` 的 `Prompts`（**纯数据，没有端口**）：装载一次之后只答不问，
//! 所以这里不再包一层结构体，直接把 `api::Prompt` 挂到它身上——**持有者因此全进程只有一处**
//! （conductor 的 `Box<dyn Prompt>`），别的能力按名字取段或拿走两块共享记录（批次 17 的收口）。
//!
//! 加载机制在 `detail/yaml_prompts.rs`；本文件不碰文件系统。

use crate::capabilities::prompt::api::{Prompt, RefsPrompts, Segment, ToolTexts, Vars};
use crate::capabilities::prompt::domain::prompt::{render, Prompts};
use crate::capabilities::prompt::ports::PromptSource;
use std::sync::Arc;

impl Prompt for Prompts {
    fn text(&self, seg: Segment) -> &str {
        self.core.segment(seg)
    }

    fn render(&self, seg: Segment, vars: Vars) -> String {
        render(self.core.segment(seg), vars)
            .expect("提示词渲染失败：变量缺失属于装配错误，须修复 prompts/ 或调用方")
    }

    fn tools(&self) -> Arc<ToolTexts> {
        Arc::clone(&self.tools)
    }

    fn refs(&self) -> Arc<RefsPrompts> {
        Arc::clone(&self.refs)
    }
}

/// 组合根专用：装载册子（缺文件 / 缺键 = 装配错误，如实报错）。
pub fn load(source: &dyn PromptSource) -> Result<Arc<dyn Prompt>, String> {
    Ok(Arc::new(source.load()?))
}
