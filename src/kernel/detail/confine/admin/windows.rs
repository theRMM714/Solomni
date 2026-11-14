//! 目的：`admin` 的 Windows 后端——台账、授权释放与机器级预授的真实实现。
//! 管：把管理动作转交给 `confine/windows/` 的实现。
//! 不管：跨平台降级文案（在 `admin/other.rs`）；一次围栏准备的步骤。
//! 联动：由 `src/kernel/detail/confine/admin.rs` 按平台选中。

use super::super::ledger::{Ledger, ReconcileReport};
use super::super::prep::MachineGrantReport;
use super::super::windows;
use crate::kernel::api::FenceSpec;
use std::path::Path;

/// 目的：按会话租约撤 ACE、删 profile。
pub(super) fn release_by_lease(home: &Path, lease: &str) -> Result<(), String> {
    windows::release_by_lease(home, lease)
}

/// 目的：一次性调用的三步收尾各记错、合并返回。
pub(super) fn release_call(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();
    if let Err(e) = super::super::release_fence(spec, home) {
        errors.push(e);
    }
    let name = windows::container_name(spec);
    if let Err(e) = windows::remove_profile_one(home, &name) {
        errors.push(e);
    }
    if let Some(dir) = windows::container_package_dir(spec)
        && dir.is_dir()
        && let Err(e) = std::fs::remove_dir_all(&dir)
    {
        errors.push(format!("删按次临时目录失败（{}）：{}", dir.display(), e));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

/// 目的：启动期按台账回收陈旧授权。
pub(super) fn reconcile(home: &Path) -> ReconcileReport {
    windows::reconcile(home)
}

/// 目的：按容器 SID 族清扫台账之外的孤儿 ACE。
pub(super) fn sweep_orphan_aces(root: &Path) -> Result<usize, String> {
    windows::sweep_orphan_aces(root)
}

/// 目的：读台账现值。
pub(super) fn ledger(home: &Path) -> Ledger {
    windows::catalog(home)
}

/// 目的：按路径只还原一条快照。
pub(super) fn restore_one(home: &Path, path: &Path) -> Result<String, String> {
    windows::restore_one(home, path)
}

/// 目的：按 SID + 路径只撤一条授权。
pub(super) fn revoke_grant(home: &Path, sid: &str, path: &Path) -> Result<String, String> {
    windows::revoke_grant(home, sid, path)
}

/// 目的：按名只删一个容器 profile。
pub(super) fn remove_profile_one(home: &Path, name: &str) -> Result<String, String> {
    windows::remove_profile_one(home, name)
}

/// 目的：撤销一次调用的围栏授权。
pub(super) fn release_fence(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    windows::release_fence(spec, home)
}

/// 目的：清扫本程序建过的整族容器 profile。
pub(super) fn sweep_profiles() -> Result<usize, String> {
    windows::sweep_profiles()
}

/// 目的：一次性的机器级预授还缺哪些落点。
pub(super) fn machine_missing(root: &Path) -> Result<Vec<String>, String> {
    windows::machine_missing(root)
}

/// 目的：把机器级落点一次性授好。
pub(super) fn machine_ensure(
    root: &Path,
    home: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    windows::machine_ensure(root, home, out)
}

/// 目的：退役台账里旧的机器级授权。
pub(super) fn retire_legacy_machine_grants(
    home: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    windows::retire_legacy_machine_grants(home, out)
}

/// 目的：给外部工作区授机器级一次性只读。
pub(super) fn ensure_workspace_grant(
    home: &Path,
    root: &Path,
) -> Result<MachineGrantReport, String> {
    windows::ensure_workspace_grant(home, root)
}

/// 目的：撤掉外部工作区的机器级只读授权。
pub(super) fn revoke_machine_grants(home: &Path, path: &Path) -> Result<usize, String> {
    windows::revoke_machine_grants(home, path)
}

/// 目的：全量回收。
pub(super) fn clean_all(home: &Path) -> Result<String, String> {
    windows::clean_all(home)
}
