//! 目的：`admin` 的非 Windows 后端——没有本机权限项，管理动作如实降级。
//! 管：返回空报告、零计数或「本平台没有该机制」的错误。
//! 不管：Windows 的 ACL、profile 与台账。
//! 联动：由 `src/kernel/detail/confine/admin.rs` 在非 Windows 目标选中。

use super::super::ledger::{Ledger, ReconcileReport};
use super::super::prep::MachineGrantReport;
use crate::kernel::api::FenceSpec;
use std::path::Path;

/// 目的：非 Windows 没有持久授权，释放为空操作。
pub(super) fn release_by_lease(_home: &Path, _lease: &str) -> Result<(), String> {
    Ok(())
}

/// 目的：非 Windows 没有按次 profile 与 ACE，收尾为空操作。
pub(super) fn release_call(_spec: &FenceSpec, _home: &Path) -> Result<(), String> {
    Ok(())
}

/// 目的：非 Windows 围栏不留权限项，如实返回空报告。
pub(super) fn reconcile(_home: &Path) -> ReconcileReport {
    ReconcileReport {
        note: "本平台的围栏不留权限项，没有台账可对账".to_string(),
        ..Default::default()
    }
}

/// 目的：非 Windows 没有可清扫的 ACL。
pub(super) fn sweep_orphan_aces(_root: &Path) -> Result<usize, String> {
    Ok(0)
}

/// 目的：非 Windows 没有台账。
pub(super) fn ledger(_home: &Path) -> Ledger {
    Ledger {
        note: "本平台的围栏不留权限项，没有台账".to_string(),
        entries: Vec::new(),
    }
}

/// 目的：非 Windows 没有可还原的快照。
pub(super) fn restore_one(_home: &Path, _path: &Path) -> Result<String, String> {
    Err("本平台的围栏不留权限项，没有可还原的快照".to_string())
}

/// 目的：非 Windows 没有可撤销的授权。
pub(super) fn revoke_grant(_home: &Path, _sid: &str, _path: &Path) -> Result<String, String> {
    Err("本平台的围栏不留权限项，没有可撤销的授权".to_string())
}

/// 目的：非 Windows 没有容器 profile。
pub(super) fn remove_profile_one(_home: &Path, _name: &str) -> Result<String, String> {
    Err("本平台没有容器 profile".to_string())
}

/// 目的：非 Windows 没有持久授权可释放。
pub(super) fn release_fence(_spec: &FenceSpec, _home: &Path) -> Result<(), String> {
    Ok(())
}

/// 目的：非 Windows 没有容器 profile 可扫。
pub(super) fn sweep_profiles() -> Result<usize, String> {
    Ok(0)
}

/// 目的：非 Windows 没有机器级 ACL 预授。
pub(super) fn machine_missing(_root: &Path) -> Result<Vec<String>, String> {
    Err("本平台没有机器级 ACL 预授（已登记缺口）".to_string())
}

/// 目的：非 Windows 没有机器级 ACL 预授。
pub(super) fn machine_ensure(
    _root: &Path,
    _home: &Path,
    _out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    Err("本平台没有机器级 ACL 预授（已登记缺口）".to_string())
}

/// 目的：非 Windows 没有旧机器级授权可退役。
pub(super) fn retire_legacy_machine_grants(
    _home: &Path,
    _out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    Ok(())
}

/// 目的：非 Windows 没有 ACL 步骤，返回零报告。
pub(super) fn ensure_workspace_grant(
    _home: &Path,
    _root: &Path,
) -> Result<MachineGrantReport, String> {
    Ok(MachineGrantReport::default())
}

/// 目的：非 Windows 没有 ACL 可撤。
pub(super) fn revoke_machine_grants(_home: &Path, _path: &Path) -> Result<usize, String> {
    Ok(0)
}

/// 目的：非 Windows 围栏不留权限项，无需清理。
pub(super) fn clean_all(_home: &Path) -> Result<String, String> {
    Ok("本平台的围栏不留权限项，无需清理".to_string())
}
