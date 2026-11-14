//! 目的：`http_tls` 的非 Windows 后端——rustls + ring + 系统证书验证器。
//! 管：把预配置的 rustls 客户端配置注入 reqwest 构建器。
//! 不管：Windows 的 native-tls；HTTP 语义。
//! 联动：由 `src/capabilities/llm/detail/http_tls.rs` 在非 Windows 目标选中。

/// 目的：注入与 ureq 口径一致的 rustls 配置。
pub(crate) fn configure(builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, String> {
    Ok(builder.use_preconfigured_tls(tls_config()?))
}

/// 目的：非 Windows 的 rustls + ring 提供者 + 系统证书验证器。
fn tls_config() -> Result<rustls::ClientConfig, String> {
    use rustls_platform_verifier::ConfigVerifierExt;
    let _ = rustls::crypto::ring::default_provider().install_default();
    rustls::ClientConfig::with_platform_verifier().map_err(|e| format!("TLS 配置失败：{}", e))
}
