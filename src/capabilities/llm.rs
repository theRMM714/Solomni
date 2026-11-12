//! 目的：llm 能力——一次会话的端口族、协议类型与回复信封解析。
//! 管：模块根；声明 api / detail / domain / ports / service 的边界。
//! 不管：会话与协作（在 conductor）；模型选择与路由（在 registry）；IO 机制（在 detail）。
//! 联动：入站面 api.rs；出站端口 ports.rs；组合根装配 service.rs 与 detail。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
