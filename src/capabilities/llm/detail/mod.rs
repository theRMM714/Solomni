//! **细节实现 = 本能力自己的适配器**：只能由**入口层的组合根**构造（门禁判定）。

pub mod endpoint;
pub mod fake_chat;
pub mod http_agent;
pub mod http_chat;
pub mod http_probe;
pub mod model_catalog;
pub mod repair;

pub use http_chat::HttpGateway;
pub use model_catalog::HttpModelCatalog;
pub use repair::UnambiguousRepair;
