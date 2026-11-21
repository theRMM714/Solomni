//! 目的：`prep` 的 Windows 后端——围栏授权准备、按次回收与机器级预授。
//! 管：把 `confine/windows/` 的实现转交给门面。
//! 不管：台账条目形状（在 `ledger`）；非 Windows 的降级文案（在 `prep/other.rs`）。
//! 联动：由 `src/kernel/detail/confine/prep.rs` 按平台选中。

use super::FencePrep;
use crate::kernel::contracts::fence::FenceSpec;
use crate::kernel::detail::confine::windows as backend;

/// 目的：把围栏要用的授权一次性做好（写目录 ACL）。
pub fn prepare_fence(spec: &FenceSpec, command: &str, home: &std::path::Path) -> FencePrep {
    backend::prepare_fence(spec, command, home)
}

/// 目的：默认回收：按台账撤掉按次 / 会话权限项与容器 profile，保留机器级授权（仅测试用）。
#[cfg(test)]
pub fn clean(home: &std::path::Path) -> Result<String, String> {
    backend::clean(home)
}
/// 目的：一次性的**机器级**预授还缺哪些落点（空 = 已就绪）。只读、O(深度 + 2)，启动对账用。
pub fn machine_missing(root: &std::path::Path) -> Result<Vec<String>, String> {
    backend::machine_missing(root)
}

/// 目的：把机器级落点一次性授好（幂等；每条进度交给 `out`）。`--setup=fence` 与首次启动授权共用。
pub fn machine_ensure(
    root: &std::path::Path,
    home: &std::path::Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    backend::machine_ensure(root, home, out)
}

/// 目的：退役台账里旧的全应用包**机器级**授权（`--setup=fence` 顺带做的一次性外部状态清理）。
///   先授后撤：能力 SID 的 ACE 没覆盖那条权限位就保留；系统目录自带的 ACE 不在台账里，一条都不碰。
pub fn retire_legacy_machine_grants(
    home: &std::path::Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    backend::retire_legacy_machine_grants(home, out)
}
