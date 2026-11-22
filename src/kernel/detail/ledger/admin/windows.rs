//! 目的：`admin` 的 Windows 后端——台账与授权释放的真实实现。
//! 管：把管理动作转交给 `ledger/windows/` 与 `container` 的实现。
//! 不管：跨平台降级文案（在 `admin/other.rs`）、机器级预授（在 `confine`）；一次围栏准备的步骤。
//! 联动：由 `src/kernel/detail/ledger/admin.rs` 按平台选中。

use super::super::windows;
use super::super::{Ledger, ReconcileReport};
use crate::kernel::contracts::fence::FencePlan;
use std::path::Path;

/// 目的：按会话租约撤 ACE、删 profile。
pub(super) fn release_by_lease(home: &Path, lease: &str) -> Result<(), String> {
    windows::release_by_lease(home, lease)
}

/// 目的：一次性调用的三步收尾各记错、合并返回。
pub(super) fn release_call(spec: &FencePlan, home: &Path) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();
    if let Err(e) = windows::release_fence(spec, home) {
        errors.push(e);
    }
    let name = crate::kernel::detail::container::container_name(spec);
    if let Err(e) = windows::remove_profile_one(home, &name) {
        errors.push(e);
    }
    if let Some(dir) = crate::kernel::detail::container::container_package_dir(spec)
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

/// 目的：清扫本程序建过的整族容器 profile。
pub(super) fn sweep_profiles() -> Result<usize, String> {
    windows::sweep_profiles()
}

/// 目的：全量回收。
pub(super) fn clean_all(home: &Path) -> Result<String, String> {
    windows::clean_all(home)
}
