//! 目的：围栏台账与管理动作的平台分派——哪些动作有真实机制、哪些平台如实降级。
//! 管：按平台选中台账读写、授权释放、机器级预授与清理的实现。
//! 不管：台账条目的形状（在 `ledger.rs`）、一次围栏准备的步骤（在 `prep.rs`）。
//! 联动：由 `ledger.rs` / `prep.rs` / `confine.rs` 调用。

use super::ledger::{Ledger, ReconcileReport};
use super::prep::MachineGrantReport;
use crate::kernel::api::FenceSpec;
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other as backend;

/// 目的：按会话租约释放授权与 profile；没有该机制的平台为空操作。
pub(super) fn release_by_lease(home: &Path, lease: &str) -> Result<(), String> {
    backend::release_by_lease(home, lease)
}

/// 目的：一次性调用的收尾（撤 ACE、删 profile 与临时目录）；没有该机制的平台为空操作。
pub(super) fn release_call(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    backend::release_call(spec, home)
}

/// 目的：启动期按台账回收陈旧授权。
pub(super) fn reconcile(home: &Path) -> ReconcileReport {
    backend::reconcile(home)
}

/// 目的：按容器 SID 族清扫台账之外的孤儿 ACE。
pub(super) fn sweep_orphan_aces(root: &Path) -> Result<usize, String> {
    backend::sweep_orphan_aces(root)
}

/// 目的：读台账现值。
pub(super) fn ledger(home: &Path) -> Ledger {
    backend::ledger(home)
}

/// 目的：按路径只还原一条快照。
pub(super) fn restore_one(home: &Path, path: &Path) -> Result<String, String> {
    backend::restore_one(home, path)
}

/// 目的：按 SID + 路径只撤一条授权。
pub(super) fn revoke_grant(home: &Path, sid: &str, path: &Path) -> Result<String, String> {
    backend::revoke_grant(home, sid, path)
}

/// 目的：按名只删一个容器 profile。
pub(super) fn remove_profile_one(home: &Path, name: &str) -> Result<String, String> {
    backend::remove_profile_one(home, name)
}

/// 目的：撤销一次调用的围栏授权。
pub(super) fn release_fence(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    backend::release_fence(spec, home)
}

/// 目的：清扫本程序建过的整族容器 profile。
pub(super) fn sweep_profiles() -> Result<usize, String> {
    backend::sweep_profiles()
}

/// 目的：一次性的机器级预授还缺哪些落点。
pub(super) fn machine_missing(root: &Path) -> Result<Vec<String>, String> {
    backend::machine_missing(root)
}

/// 目的：把机器级落点一次性授好（幂等；每条进度交给 `out`）。
pub(super) fn machine_ensure(
    root: &Path,
    home: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    backend::machine_ensure(root, home, out)
}

/// 目的：退役台账里旧的机器级授权。
pub(super) fn retire_legacy_machine_grants(
    home: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    backend::retire_legacy_machine_grants(home, out)
}

/// 目的：给一个外部工作区授机器级一次性只读。
pub(super) fn ensure_workspace_grant(
    home: &Path,
    root: &Path,
) -> Result<MachineGrantReport, String> {
    backend::ensure_workspace_grant(home, root)
}

/// 目的：撤掉某个外部工作区的机器级只读授权。
pub(super) fn revoke_machine_grants(home: &Path, path: &Path) -> Result<usize, String> {
    backend::revoke_machine_grants(home, path)
}

/// 目的：全量回收（连机器级授权一起撤）。
pub(super) fn clean_all(home: &Path) -> Result<String, String> {
    backend::clean_all(home)
}
