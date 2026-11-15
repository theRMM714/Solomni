//! 目的：proxy 能力的入站契约：执行者装配面、回调端口与领域类型的唯一出口。
//! 管：重导出 `ProxyHost`、`ProxyTools` / `ProxyHandler` 与 `domain::proxy` 的全部公开类型。
//! 不管：实现（`service`）、纯规则（`domain`）、宿主与装配（`conductor` 的队列桥、组合根）。
//! 联动：`conductor` 实现 `ProxyHost` 并经本模块构造 `ProxyHandler`；测试替身只经本模块。

pub use crate::capabilities::proxy::domain::proxy::*;
pub use crate::capabilities::proxy::ports::ProxyHost;
#[allow(unused_imports)] // 入站面：其它业务与测试都经这里取执行者
pub use crate::capabilities::proxy::service::proxy::{ProxyHandler, ProxyTools};
