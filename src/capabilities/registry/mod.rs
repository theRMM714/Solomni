//! 目的：registry 能力——供应商 / 模型 / agent / 基本设置的内存形态与解析（四份 yaml）。
//! 管：模块根；声明 api / detail / domain / ports / service 的边界；密钥只在本能力与核心发起的出站调用。
//! 不管：落盘机制（在 detail/yaml_settings.rs）；llm 通道会话（在 llm 能力）；用户私有区布局（在 kernel）。
//! 联动：入站面 api.rs；出站端口 ports.rs；conductor 持有 Box<dyn Registry>。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
