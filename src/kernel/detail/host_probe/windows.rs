//! 目的：`host_probe` 的 Windows 虚拟化事实——系统目录里有没有 `WinHvPlatform.dll`。
//! 管：虚拟机监视器是否在场的只读判定。
//! 不管：起虚拟机、能力等级与降级策略。
//! 联动：由 `src/kernel/detail/host_probe.rs` 按平台选中。

use std::path::Path;

/// 目的：系统目录里存在 `WinHvPlatform.dll` 即视为可用。
pub(crate) fn available() -> bool {
    std::env::var_os("SystemRoot")
        .map(|root| {
            Path::new(&root)
                .join("System32")
                .join("WinHvPlatform.dll")
                .is_file()
        })
        .unwrap_or(false)
}
