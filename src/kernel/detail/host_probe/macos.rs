//! 目的：`host_probe` 的 macOS 虚拟化事实——系统自带虚拟化框架。
//! 管：虚拟机监视器是否在场的只读判定。
//! 不管：起虚拟机、能力等级与降级策略。
//! 联动：由 `src/kernel/detail/host_probe.rs` 按平台选中。

/// 目的：macOS 11+ 都带虚拟化框架，恒为可用。
pub(crate) fn available() -> bool {
    true
}
