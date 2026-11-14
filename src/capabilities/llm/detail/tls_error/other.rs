//! 目的：`tls_error` 的非 Windows 后端——没有 native-tls 专属变体。
//! 管：如实返回 None。
//! 不管：通用错误分类；文案。
//! 联动：由 `src/capabilities/llm/detail/tls_error.rs` 在非 Windows 目标选中。

/// 目的：非 Windows 没有平台专属 TLS 变体。
pub(crate) fn classify_native(_e: &ureq::Error, _lacks: bool) -> Option<&'static str> {
    None
}
