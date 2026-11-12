//! 目的：模型通道的纯逻辑——把回复解析成信封（发言 / 表态 / 工具调用）。
//! 管：`envelope` / `malformed` 两个子模块。
//! 不管：出站请求、TLS 与模型目录（在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费；协作与代理两条路经 `api.rs` 用它。

pub mod envelope;
pub mod malformed;
