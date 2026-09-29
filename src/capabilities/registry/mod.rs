//! 登记处能力：供应商 / 模型 / agent / 基本设置的**内存形态与解析**（四份 yaml）。
//!
//! 密钥只存在于这里与核心发起的出站调用：对外视图（`ProviderView`）永不携带密钥。
//! 持久化机制在适配层（`ports::SettingsStore`）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
