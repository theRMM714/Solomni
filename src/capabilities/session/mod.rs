//! 会话能力：一个 agent 会话的**状态与簿记**、转录行的线格式、事件词汇与历史视图。
//!
//! 它只回答"这场对话里发生过什么"：对话、行 id / 回合 / 回复簿记、压缩点、回档边界。
//! **回合驱动不在它这里**（在 `engine`：驱动必须能读写会话状态，方向是 engine → session）；
//! 落盘机制在适配层（`ports::HistoryStore`）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
