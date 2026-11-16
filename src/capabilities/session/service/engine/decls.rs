//! 目的：工具声明面的值类型——线上名回译表、一次请求的工具表，与成员这次请求的声明面。
//! 管：`WireTools` / `Decls` / `ToolDecls` 三个纯值类型。
//! 不管：声明怎么算出来（`tools.rs` 的 `tool_decls`）；声明怎么发给供应商（`round.rs` 的 `assemble`）。
//! 联动：`round` 与 `tools` 各取同一份，避免两边互相引用。

use std::collections::BTreeMap;
/// 目的：线上名 → (模块 id, 工具名)：原生协议里没有 module 字段，跨模块同名工具靠它消歧。
pub type WireTools = BTreeMap<String, (Option<String>, String)>;

/// 目的：一次请求要声明的工具表（给供应商的那一份）。
pub type Decls = Vec<crate::capabilities::llm::api::ToolDecl>;

/// 目的：本成员这次请求的**工具声明面**（原生通道用）：发给供应商的声明 + 线上名回译表 + 可并发的线上名。
#[derive(Default)]
pub struct ToolDecls {
    pub list: Decls,
    /// 目的：线上名 → (模块 id, 工具名)：原生协议里没有 module 字段，跨模块同名工具靠它消歧。
    pub wire: WireTools,
}
