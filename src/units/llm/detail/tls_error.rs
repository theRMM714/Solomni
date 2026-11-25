//! 目的：按平台识别 ureq 的 TLS 错误家族——Windows 的 native-tls 走独立变体。
//! 管：把平台专属的错误变体翻译成与通用 `Tls` 相同的性质结论。
//! 不管：错误性质的通用分类（在 `http_agent::classify`）；文案与脱敏。
//! 联动：`http_agent::classify` 在通用匹配未命中时调用本模块。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other as backend;

/// 目的：平台专属 TLS 错误变体的分类；不是该变体给 None。
/// 参数：`lacks` 表示错误原文里含「取不到系统凭证」的稳定码。
pub(crate) fn classify_native(e: &ureq::Error, lacks: bool) -> Option<&'static str> {
    backend::classify_native(e, lacks)
}
