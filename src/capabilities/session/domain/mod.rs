//! 纯逻辑：会话状态与簿记、转录行与事件词汇、历史视图。没有 IO，也不加 trait。

pub mod events;
pub mod history;
pub mod rewind;
pub mod session;
