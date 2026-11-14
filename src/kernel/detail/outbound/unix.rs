//! 目的：`outbound` 的非 Windows 后端——交给 rustls 默认配置。
//! 管：TLS 配置（None 表示用 rustls 默认）与后端名称。
//! 不管：代理构建的通用参数（在门面）；协议与错误语义。
//! 联动：由 `src/kernel/detail/outbound.rs` 在非 Windows 目标选中。

/// 目的：非 Windows 不额外注入 TLS 配置，用 rustls 默认。
pub(crate) fn tls_config() -> Option<ureq::tls::TlsConfig> {
    None
}

/// 目的：如实报出本平台实际使用的 TLS 后端。
pub(crate) fn tls_backend() -> &'static str {
    "rustls"
}
