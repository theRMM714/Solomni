//! 目的：llm 单元的细节实现（适配器）模块根。
//! 管：声明适配器子模块（endpoint / protocol / http_chat / http_stream / http_probe / http_agent / model_catalog / repair，测试用的 fake_chat）；重导出 HttpGateway / HttpModelCatalog / UnambiguousRepair。
//!     protocol 是**唯一认识供应商 wire 字段名**的目录；其余适配器只管出站机制。
//! 不管：对外契约（在 api / ports）；域内纯逻辑（在 domain）；装配决策（在组合根）。
//! 联动：只能由入口层组合根构造（门禁判定）；实现本单元的出站端口。

pub mod endpoint;
#[cfg(test)]
pub mod fake_chat;
pub mod http_agent;
pub mod http_chat;
pub mod http_probe;
pub(crate) mod http_stream;
pub mod model_catalog;
pub(crate) mod protocol;
pub mod repair;
pub(crate) mod tls_error;

pub use http_chat::HttpGateway;
pub use model_catalog::HttpModelCatalog;
pub use repair::UnambiguousRepair;
