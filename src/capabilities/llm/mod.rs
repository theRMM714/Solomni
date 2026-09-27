//! 模型通道能力：一次会话的端口族、协议类型与**回复信封解析**。
//!
//! 它不认识会话、不认识协作：只把"发给供应商的消息"与"供应商回来的文本"这两件事说清。
//! 信封解析（`domain/envelope`）是纯逻辑——把模型回复的最外层结构认出来（发言 / 表态 / 工具调用）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
