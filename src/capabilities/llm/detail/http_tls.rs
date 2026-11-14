//! 目的：reqwest 客户端的平台 TLS 装配。
//! 管：按平台决定是否注入预配置 TLS（非 Windows 用 rustls + 系统证书验证器）。
//! 不管：HTTP 客户端预算、请求与响应语义。
//! 联动：`http_stream::client` 构建客户端前调用本模块。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as backend;

/// 目的：按平台配置客户端构建器；配置失败如实返回错误。
pub(crate) fn configure(builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, String> {
    backend::configure(builder)
}
