//! 入站能力面：**其它能力、conductor 与呈现层只准用这里**（不许碰 `domain` / `ports`）。
//!
//! 两样东西在这里：
//! - **能力面 `Prompt`**：按名字取段（`Segment`），或拿走两块**共享记录**（`tools()` / `refs()`）。
//!   **册子的布局只有本能力知道**——别的能力不点字段路径。
//! - **工具文案与引用文案的 DTO**：它们在 tools / workspace / collab 的签名里当家。
//!
//! 持有者只有 `service.rs` 一处；组合根装一次，conductor 只持 `Box<dyn Prompt>`。

pub use crate::capabilities::prompt::domain::prompt::{Segment, Vars};
pub use crate::capabilities::prompt::domain::refs::{rewrite, RefRoots};

/// 提示词册能力：按名字答"某一段是什么"，并共享两块大记录。
pub trait Prompt: Send + Sync {
    /// 按名字取一段**原文**（整段发给模型、或调用方要自己拼时用）。
    fn text(&self, seg: Segment) -> &str;

    /// 按名字渲染一段（`{{key}}` 替换）；**缺变量 = 装配错误**，直接暴露，不静默兜底。
    fn render(&self, seg: Segment, vars: Vars) -> String;

    /// 按（**会话使用类型**，**角色**）取这个回合的机制说明：只拿同时匹配的条目，
    /// 按册子声明顺序拼接（没配 = 空串）。对应关系在 `prompts/shared/mechanisms.yaml`，这里只做匹配。
    fn mechanism(&self, session: &str, role: &str) -> String;

    /// 按（**宿主平台**，**guest 的 os**）取工作环境块并渲染：虚拟机档传 guest，本机档传 None。
    /// 对应关系在 `prompts/shared/tools.yaml` 的 `envs`，这里只做匹配；变量与 `render` 同一套。
    fn env(&self, host: &str, guest: Option<&str>, vars: Vars) -> String;
}
