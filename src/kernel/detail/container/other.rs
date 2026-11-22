//! 目的：非 Windows 平台的容器面占位——回环中继只有 Windows 需要（Unix 的回环本来就通，见 `NetPolicy::Local` 的注释）。
//! 管：内部协议 `--fence-relay` 在非 Windows 上的如实拒绝。
//! 不管：围栏本身（Linux 在 `confine/linux.rs`、macOS 在 `confine/macos.rs`）。
//! 联动：`src/entry/args.rs` 把 `--fence-relay` 分派到 `relay_entry`；本文件只在非 Windows 编译。

use crate::kernel::contracts::fence::FENCE_FAILED;

/// 目的：内部协议旗（与 Windows 版同一个字面量，分派在 `src/entry/args.rs`）。
pub(crate) const RELAY_FLAG: &str = "--fence-relay";

/// 目的：非 Windows 上不该出现这个旗（工具面在非 Windows 不派 `NetPolicy::Local`）；真出现了就如实拒绝。
pub(crate) fn relay_entry(args: &[String]) -> i32 {
    eprintln!(
        "[中继] 本平台没有回环中继：收到 {} 个参数；这次调用没有执行。",
        args.len()
    );
    FENCE_FAILED
}
