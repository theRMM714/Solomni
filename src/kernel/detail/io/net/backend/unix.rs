//! 目的：`backend` 的非 Windows 后端——两条通道都用 rustls + ring + 系统证书验证器。
//! 管：ureq 的 TLS 留空（交给 rustls 默认）与 reqwest 的预配置注入。
//! 不管：ureq 代理的预算与状态语义；HTTP 协议与错误归类。
//! 联动：由 `src/kernel/detail/io/net/backend.rs` 在非 Windows 目标选中。

/// 目的：非 Windows 的 ureq 用 rustls 默认，无需额外配置。
pub(crate) fn ureq_tls() -> Option<ureq::tls::TlsConfig> {
    None
}

/// 目的：把与 ureq 同口径的 rustls 配置注入 reqwest 构建器。
pub(crate) fn reqwest_builder(
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::ClientBuilder, String> {
    Ok(builder.use_preconfigured_tls(rustls_config()?))
}

/// 目的：rustls + ring 提供者 + 系统证书验证器（与 ureq 的 rustls 后端同一信任来源）。
fn rustls_config() -> Result<rustls::ClientConfig, String> {
    use rustls_platform_verifier::ConfigVerifierExt;
    let _ = rustls::crypto::ring::default_provider().install_default();
    rustls::ClientConfig::with_platform_verifier().map_err(|e| format!("TLS 配置失败：{}", e))
}

/// 目的：如实报出本平台两条通道实际使用的 TLS 后端。
pub(crate) fn name() -> &'static str {
    "rustls"
}
