//! 目的：出站 HTTPS 的共享机制——按平台装配 TLS 的 ureq 代理。
//! 管：**唯一**一处 TLS 后端装配（Windows 显式 native-tls / 其余 rustls）与通用代理构建。
//! 不管：具体协议、状态码与错误语义（在各自能力里）。
//! 联动：llm 的 http_agent 与依赖补齐都经它取代理 / TLS 配置。

use std::time::Duration;

/// 目的：本平台该用的 TLS 配置——Windows 必须显式 native-tls（ureq 3 不会自动选中），其余平台给 None（用 rustls 默认）。
pub fn tls_config() -> Option<ureq::tls::TlsConfig> {
    #[cfg(windows)]
    {
        Some(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                // native-tls 的 PlatformVerifier = 系统证书库。
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 目的：构建带连接 / 总超时的通用代理。
/// 约束：4xx/5xx 当**数据**不当错误（`http_status_as_error(false)`），状态由调用方判——错误文案里才留得住对方原话。
pub fn agent(connect_secs: u64, total_secs: u64) -> ureq::Agent {
    let mut builder = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(connect_secs)))
        .timeout_global(Some(Duration::from_secs(total_secs)))
        .http_status_as_error(false);
    if let Some(tls) = tls_config() {
        builder = builder.tls_config(tls);
    }
    builder.build().into()
}

/// 目的：这条构建实际用的 TLS 后端（编译期事实）：探针与自检据此如实报出来，不靠人猜。
pub fn tls_backend() -> &'static str {
    if cfg!(windows) {
        "native-tls"
    } else {
        "rustls"
    }
}
