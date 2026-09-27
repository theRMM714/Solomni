//! 入站能力面：**其它能力、core 与呈现层只准用这里**（不许碰 `domain` / `ports`）。
//!
//! 两样东西在这里：
//! - **能力面 `Prompt`**：按名字取段（`Segment`），或拿走两块**共享记录**（`tools()` / `refs()`）。
//!   **册子的布局只有本能力知道**——别的能力不点字段路径（批次 17 的收口）。
//! - **工具文案与引用文案的 DTO**：它们在 tools / workspace / collab 的签名里当家。
//!
//! 持有者只有 `service.rs` 一处；组合根装一次，core 只持 `Box<dyn Prompt>`。

pub use crate::capabilities::prompt::domain::prompt::{RefsPrompts, Segment, ToolTexts, Vars};
pub use crate::capabilities::prompt::domain::refs::{rewrite, RefRoots};

use std::sync::Arc;

/// 提示词册能力：按名字答"某一段是什么"，并共享两块大记录。
pub trait Prompt: Send + Sync {
    /// 按名字取一段**原文**（整段发给模型、或调用方要自己拼时用）。
    fn text(&self, seg: Segment) -> &str;

    /// 按名字渲染一段（`{{key}}` 替换）；**缺变量 = 装配错误**，直接暴露，不静默兜底。
    fn render(&self, seg: Segment, vars: Vars) -> String;

    /// 工具与路径的模型侧文案（~100 条模板）：**共享一份**——调用方拿 `Arc`，不深拷贝。
    fn tools(&self) -> Arc<ToolTexts>;

    /// `@` 引用的两句说明文案：共享一份。
    fn refs(&self) -> Arc<RefsPrompts>;
}
