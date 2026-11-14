//! 目的：`host_probe` 的 Linux 虚拟化事实——`/dev/kvm` 是否存在。
//! 管：虚拟机监视器是否在场的只读判定。
//! 不管：起虚拟机、能力等级与降级策略。
//! 联动：由 `src/kernel/detail/host_probe.rs` 按平台选中。

use std::path::Path;

/// 目的：`/dev/kvm` 存在即视为可用。
pub(crate) fn available() -> bool {
    Path::new("/dev/kvm").exists()
}
