//! 目的：`http_tls` 的 Windows 后端——reqwest 的 native-tls 由依赖特性决定，无需再注入。
//! 管：构建器原样返回。
//! 不管：rustls 配置；HTTP 语义。
//! 联动：由 `src/capabilities/llm/detail/http_tls.rs` 按平台选中。

/// 目的：Windows 不额外配置 TLS。
pub(crate) fn configure(builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, String> {
    Ok(builder)
}
