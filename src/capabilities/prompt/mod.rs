//! 提示词能力：册子的内存形态、`{{key}}` 渲染、`@` 引用改写。
//!
//! 纯逻辑（没有 IO）：册子从哪来由 `ports::PromptSource` 决定，文件机制在适配层。
//! 对外只有 `api`；`domain` 与 `ports` 是本能力内部（门禁会拦越界引用）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
