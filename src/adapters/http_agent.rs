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

/// 这条构建实际用的 TLS 后端（编译期事实）：探针与自检据此如实报出来，不靠人猜。
pub fn tls_backend() -> &'static str {
    if cfg!(windows) {
        "native-tls"
    } else {
        "rustls"
    }
}

/// 本进程**取不到系统 TLS 凭证**时的稳定错误码（Windows schannel 的 `SEC_E_NO_CREDENTIALS`）。
/// 判据只认码，不认文案：错误消息是本地化的，按文案匹配会在别的语言环境下失灵。
/// 现场：沙箱把工作区外的用户凭证存储挡住时，连 curl.exe 都握不了手；放开沙箱后同一条链路立刻通。
const SEC_E_NO_CREDENTIALS: i64 = 0x8009_030E_u32 as i64;

/// 出站错误的**性质**分类（只用于如实区分"环境连不上外网"与"我们自己的链路坏了"）：
/// - 网络类（Io / 域名解析不到 / 连接失败 / 超时）→ `no-net`：环境问题，探针据此 env-skip；
/// - TLS 错误里**明确是"本进程取不到系统凭证"**那一码 → `env-tls`：这是本机/本进程的环境结论，
///   不是"我们链路坏了"——但判据必须精确到错误码，其余 TLS 错误照旧算我们的问题；
/// - 其余 TLS 错误 → 我们链路的问题，探针必须报失败（不许把 TLS 坏掉说成"环境不允许"）；
/// - 真正认不出的错误由 `_` 收 → `fail`，原始原文照打，绝不因为认不出性质就当成"环境不允许"。
///
/// **TLS 家族不止一个变体**：Windows 上 native-tls 的错误走 `NativeTls`（不是通用的 `Tls` 壳），
/// 只匹配 `Tls` 会让真机上的凭证错误落进 `_` 被报成 `fail`——真机上抓到过一次。
/// 两个变体都按同一份判据（错误原文里的稳定码）分类。
pub fn classify(e: &ureq::Error) -> &'static str {
    match e {
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Timeout(_) => "no-net",
        ureq::Error::Tls(_) if lacks_system_credentials(&e.to_string()) => "env-tls",
        #[cfg(windows)]
        ureq::Error::NativeTls(_) if lacks_system_credentials(&e.to_string()) => "env-tls",
        ureq::Error::Tls(_) => "tls-fail",
        #[cfg(windows)]
        ureq::Error::NativeTls(_) => "tls-fail",
        _ => "fail",
    }
}

/// 错误原文里有没有"取不到系统凭证"这个稳定码（十六进制与**有符号十进制**两种写法都认）。
/// 有符号那一份必须按 i32 渲染：Windows 把它印成 `-2146893042`，而按 u32 渲染是 `2148074254`——
/// 两者是同一个码，少认一个就会把环境结论误报成失败（真机上抓到过一次）。
/// 只在**已经判定是 TLS 错误**之后才问它——所以这条判据不会把别的失败误判成环境结论。
pub(crate) fn lacks_system_credentials(text: &str) -> bool {
    let hex = format!("{:08X}", SEC_E_NO_CREDENTIALS as u32);
    let signed = (SEC_E_NO_CREDENTIALS as u32 as i32).to_string();
    let upper = text.to_ascii_uppercase();
    upper.contains(&hex) || text.contains(&signed)
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
