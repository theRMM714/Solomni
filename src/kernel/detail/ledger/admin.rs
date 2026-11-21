//! 目的：台账动作的平台分派——哪些动作有真实机制、哪些平台如实降级。
//! 管：按平台选中台账读写、授权释放、profile 清扫与全量回收的实现。
//! 不管：台账条目的形状（在 `ledger.rs`）、机器级预授（在 `confine` 的 prep.rs）、一次围栏准备的步骤。
//! 联动：只由 `ledger.rs` 门面调用。

use super::{Ledger, ReconcileReport};
use crate::kernel::contracts::fence::FenceSpec;
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

/// 目的：清扫本程序建过的整族容器 profile。
pub(super) fn sweep_profiles() -> Result<usize, String> {
    backend::sweep_profiles()
}

/// 目的：全量回收（连机器级授权一起撤）。
pub(super) fn clean_all(home: &Path) -> Result<String, String> {
    backend::clean_all(home)
}
