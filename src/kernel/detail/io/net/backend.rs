//! 目的：按平台装配 TLS——ureq 与 reqwest 两条通道共用这一份决策。
//! 管：平台后端的选择与三个出口（ureq 的 TLS 配置、reqwest 构建器注入、后端名称）。
//! 不管：ureq 代理的预算与状态语义（在 `outbound`）；HTTP 协议与错误归类。
//! 联动：`outbound` 取 `ureq_tls`；`api::net` 取 `reqwest_builder` 与 `name`。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as platform;

/// 目的：ureq 通道的 TLS 配置（`None` 表示交给该栈的默认后端）。
pub(crate) fn ureq_tls() -> Option<ureq::tls::TlsConfig> {
    platform::ureq_tls()
}

/// 目的：把平台的 TLS 装配注入 reqwest 构建器；配置失败如实返回原因。
pub fn reqwest_builder(
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::ClientBuilder, String> {
    platform::reqwest_builder(builder)
}

/// 目的：两条通道实际共用的 TLS 后端（编译期事实）；探针与自检据此如实报出，不靠人猜。
pub fn name() -> &'static str {
    platform::name()
}
