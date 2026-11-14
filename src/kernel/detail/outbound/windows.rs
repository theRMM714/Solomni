//! 目的：`outbound` 的 Windows 后端——显式选 native-tls 与系统证书库。
//! 管：TLS 配置与后端名称。
//! 不管：代理构建的通用参数（在门面）；协议与错误语义。
//! 联动：由 `src/kernel/detail/outbound.rs` 按平台选中。

/// 目的：Windows 必须显式 native-tls（ureq 3 不会自动选中），证书走系统库。
pub(crate) fn tls_config() -> Option<ureq::tls::TlsConfig> {
    Some(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    )
}

/// 目的：如实报出本平台实际使用的 TLS 后端。
pub(crate) fn tls_backend() -> &'static str {
    "native-tls"
}
