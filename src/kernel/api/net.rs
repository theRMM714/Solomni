//! 目的：网络机制对外面——单元只经这里取「怎么出站」，不各自装配 TLS。
//! 管：重导出出站网络原语（ureq 代理、reqwest 构建器的 TLS 注入、本平台 TLS 后端名称）。
//! 不管：请求内容、状态码判定、重试与错误归类（各自单元）；代理与信任环境由传输栈从环境拾取。
//! 联动：实现在 `src/kernel/detail/io/net/`；llm 的 http_agent / http_stream、provision 的 fetch 与 `--https-check` 经它取。

pub use crate::kernel::detail::io::net::backend::name as tls_backend;
pub use crate::kernel::detail::io::net::backend::reqwest_builder as configure_reqwest;
pub use crate::kernel::detail::io::net::outbound::agent;
