//! 目的：`tls_error` 的 Windows 后端——native-tls 的 `NativeTls` 变体。
//! 管：把凭证缺失与其它 TLS 失败分别归入 `env-tls` / `tls-fail`。
//! 不管：通用错误分类；文案。
//! 联动：由 `src/capabilities/llm/detail/tls_error.rs` 按平台选中。

/// 目的：命中 `NativeTls` 时给出分类，其余变体给 None。
pub(crate) fn classify_native(e: &ureq::Error, lacks: bool) -> Option<&'static str> {
    match e {
        ureq::Error::NativeTls(_) if lacks => Some("env-tls"),
        ureq::Error::NativeTls(_) => Some("tls-fail"),
        _ => None,
    }
}
