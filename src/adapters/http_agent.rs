//! 出站 HTTP 代理构建（机制）：**唯一**装配 TLS 与超时的地方。
//! Windows 走系统 schannel（native-tls）——ureq 3 的 native-tls **不会自动被选中**（默认是 rustls），
//! 必须显式写进 TlsConfig，否则 HTTPS 会以"该 TLS 后端没启用"报错（旧版踩过同类坑）。
//! 非 Windows 走 rustls（默认后端，无需显式指定）。
//! ureq 3 默认把 4xx/5xx 变成**不带响应体**的 Error；这里关掉（http_status_as_error(false)），
//! 由 finish_request 统一判状态——这样错误文案里才留得住供应商自己的原话。

use std::time::Duration;

/// 构建带连接/总超时的代理；TLS 后端按平台显式装配。
pub fn agent(connect_secs: u64, total_secs: u64) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(connect_secs)))
        // 供应商接了连接却不发响应头时快速失败（ureq 3 没有"单次读取"这一档；
        // 响应体的整体预算由 timeout_global 兜底，长流式回复因此不受影响）。
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_global(Some(Duration::from_secs(total_secs)))
        // 4xx/5xx 当数据不当错误：见 finish_request。
        .http_status_as_error(false);
    #[cfg(windows)]
    let builder = builder.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            // native-tls 的 PlatformVerifier = 系统证书库（与旧版的 TlsConnector::new() 同源）。
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    );
    builder.build().into()
}

/// 出站错误里的密钥一律替换掉再出适配层（红线）。
pub fn redact(s: String, key: &str) -> String {
    if key.is_empty() {
        s
    } else {
        s.replace(key, "***")
    }
}

/// 一次出站请求的**统一收口**：把"供应商答了但状态不对"与"网络/协议错误"分开，
/// 并保证错误文案里不含密钥。返回 Err((文案, 可否换下一个候选))。
/// 为什么自己判状态：ureq 3 默认把 4xx/5xx 变成不带响应体的 Error，
/// 而我们要把供应商的原话如实回报（探测与失败诊断都靠它）。
pub fn finish_request(
    got: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    key: &str,
) -> Result<ureq::http::Response<ureq::Body>, (String, bool)> {
    match got {
        Ok(resp) if resp.status().is_success() => Ok(resp),
        Ok(resp) => {
            let code = resp.status().as_u16();
            let mut body = resp.into_body();
            let snippet: String = body
                .read_to_string()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            let msg = redact(format!("供应商返回 {}：{}", code, snippet), key);
            Err((msg, super::endpoint::retryable_status(code)))
        }
        // 网络/协议层错误：换下一个候选（端点回落）。
        Err(e) => Err((redact(format!("网络错误：{}", e), key), true)),
    }
}
