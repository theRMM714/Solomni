//! 目的：`backend` 的 Windows 后端——两条通道都用 native-tls（schannel）与系统证书库。
//! 管：ureq 的 native-tls 配置；reqwest 的 native-tls 由依赖特性定死，构建器原样返回。
//! 不管：ureq 代理的预算与状态语义；HTTP 协议与错误归类。
//! 联动：由 `src/kernel/detail/io/net/backend.rs` 按平台选中。

/// 目的：Windows 必须显式 native-tls（ureq 3 不会自动选中），证书走系统库。
pub(crate) fn ureq_tls() -> Option<ureq::tls::TlsConfig> {
    Some(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    )
}

/// 目的：reqwest 的 native-tls 由依赖特性决定，无需再注入。
pub(crate) fn reqwest_builder(
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::ClientBuilder, String> {
    Ok(builder)
}

/// 目的：如实报出本平台两条通道实际使用的 TLS 后端。
pub(crate) fn name() -> &'static str {
    "native-tls"
}
