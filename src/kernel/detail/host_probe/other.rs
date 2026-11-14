//! 目的：`host_probe` 的兜底虚拟化事实——没有已知的虚拟化机制。
//! 管：虚拟机监视器是否在场的只读判定。
//! 不管：起虚拟机、能力等级与降级策略。
//! 联动：由 `src/kernel/detail/host_probe.rs` 在其它平台选中。

/// 目的：未知平台如实报不可用。
pub(crate) fn available() -> bool {
    false
}
