//! 出站 HTTP 代理构建（机制）：Windows 走系统 schannel（native-tls），其余走 rustls。
//! ureq 的 native-tls 不在默认链路上，必须显式经 AgentBuilder::tls_connector 装上；
//! 否则 HTTPS 会落入默认分支「no TLS backend is configured」（曾实际发生）。
//! 两种后端都不需要额外动作时也走这里，保证超时设置单点。

use std::time::Duration;

/// 构建带连接/总超时的代理；TLS 后端按平台显式装配。
pub fn agent(connect_secs: u64, total_secs: u64) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(connect_secs))
        // 单次读取静默上限：供应商接了连接却不发数据时快速失败（对 SSE 也生效）
        .timeout_read(Duration::from_secs(60))
        .timeout(Duration::from_secs(total_secs));
    #[cfg(windows)]
    let builder = match ureq::native_tls::TlsConnector::new() {
        Ok(connector) => builder.tls_connector(std::sync::Arc::new(connector)),
        // 系统 TLS 初始化失败：保留默认代理，HTTPS 会以明确错误暴露，不静默兜底。
        Err(_) => builder,
    };
    builder.build()
}
