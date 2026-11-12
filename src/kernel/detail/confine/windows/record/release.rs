//! 目的：按台账收尾——把这次执行写下的授权撤掉，把根内文件的安全描述符还原。
//! 管：逐条精确撤权、整份台账的收尾、按会话租约释放与按条释放。
//! 不管：孤儿 profile 与残留 ACE 的清扫（sweep.rs），也不管按条人工处置的清单（catalog.rs）。
//! 联动：撤销经 acl.rs 的 revoke_one / restore_sd；盘上格式与锁在 store.rs；入口是 --fence-clean 与 confine/mod.rs 的 release_by_lease。

use super::super::*;
use crate::kernel::api::FenceSpec;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;

/// 目的：按 SID 字符串撤一条 ACE（先转 PSID，用完即释放）。
pub(crate) fn revoke_one_text(sid_text: &str, path: &Path, recursive: bool) -> Result<(), String> {
    let sid = sid_from_string(sid_text)?;
    let r = revoke_one(sid, path, recursive);
    free_sid(sid);
    r
}

/// 目的：精确回收——按台账把我们写过的 ACE 逐条撤掉，并删掉我们建过的容器 profile。
/// 返回：给用户看的一句话（清理了几条、删了几个 profile）。
pub fn clean(home: &Path) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let rec = load_record(home);
    if rec.is_empty() {
        return Ok("没有台账：本程序没在本机写过权限项".to_string());
    }
    let mut errors: Vec<String> = Vec::new();
    let mut restored = 0usize;
    // 根内路径整体还原原始安全描述符：被写坏的 DACL 只有这一条路能修回来。
    for snap in &rec.snapshots {
        let p = PathBuf::from(&snap.path);
        if std::fs::symlink_metadata(&p).is_err() {
            continue;
        }
        match restore_sd(&p, &snap.bytes) {
            Ok(()) => restored += 1,
            Err(e) => errors.push(e),
        }
    }
    let mut removed = 0usize;
    for grant in &rec.grants {
        // 根内路径已由上面的快照整体还原，这里不再逐条撤（重复撤一个不存在的 ACE 没有意义）。
        if rec.snapshots.iter().any(|s| s.path == grant.path) {
            continue;
        }
        let sid = match sid_from_string(&grant.sid) {
            Ok(s) => s,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let p = PathBuf::from(&grant.path);
        if std::fs::symlink_metadata(&p).is_ok() {
            match revoke_one(sid, &p, true) {
                Ok(()) => removed += 1,
                Err(e) => errors.push(e),
            }
        }
        free_sid(sid);
    }
    let mut deleted = 0usize;
    for prof in &rec.profiles {
        let n: Vec<u16> = std::ffi::OsStr::new(&prof.name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let hr = unsafe { DeleteAppContainerProfile(n.as_ptr()) };
        if hr >= 0 {
            deleted += 1;
        }
    }
    if !errors.is_empty() {
        // 还有没清掉的：台账保留，供下次重试（删了就等于把待办也删了）。
        return Err(format!(
            "台账回收未完成：{}（已还原 {} 处、已撤销 {} 条、已删 {} 个 profile；台账保留供重试）",
            errors.join("；"),
            restored,
            removed,
            deleted
        ));
    }
    // 全部清干净了才删台账；删不掉本身就是没收干净，如实报错。
    match std::fs::remove_file(record_path(home)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("权限项已清干净，但删除授权台账失败：{}", e)),
    }
    Ok(format!(
        "已还原 {} 处原始权限、撤销 {} 条授权、删除 {} 个容器 profile",
        restored, removed, deleted
    ))
}

/// 目的：撤销一次会话的授权（会话删除时经 FenceHost 端口调用）。
/// 参数：spec 是该会话各席的围栏范围；home 是产品私有区（台账落点，也是产品根的锚）。
/// 错误：撤权、还原或台账更新失败时如实返回；失败条目留在台账里，供下次重试。
pub fn release_fence(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    let sid = container_sid(&container_name(spec))?;
    let sid_text = sid_to_string(sid);
    free_sid(sid);
    // 只撤**这个会话**的归属：同名 agent 的另一个会话持有另一份 lease，不能被一起撤掉。
    let mut owner = current_owner();
    owner.lease = spec.lease.clone();
    let _lock = lock_ledger(home)?;
    // 没有台账就别凭空造一份：释放只在“确实写过授权”时才动台账文件。
    let had_record = record_path(home).exists();
    let mut rec = load_record(home);
    // 要撤的全从台账派生：这个 SID 记过的每条 (SID, 路径)。没记过的一律不动——
    // 同名 agent 的另一个实例可能正用着同一条授权，凭 spec 反推落点会把它误撤。
    let mut keys: Vec<String> = rec
        .grants
        .iter()
        .filter(|g| g.sid == sid_text)
        .map(|g| g.path.clone())
        .collect();
    keys.sort();
    keys.dedup();
    let mut errors: Vec<String> = Vec::new();
    for key in &keys {
        // 先把本次归属从这条授权里撤掉；还有别的归属（活的、或判不了的）就留着给它用。
        let (empty, recursive) = match rec
            .grants
            .iter_mut()
            .find(|g| g.sid == sid_text && g.path == *key)
        {
            Some(g) => {
                g.owners.retain(|o| *o != owner);
                (g.owners.is_empty(), g.recursive)
            }
            None => continue,
        };
        if !empty {
            continue;
        }
        finalize_grant(&mut rec, &sid_text, key, recursive, &mut errors);
    }
    if had_record {
        if let Err(e) = save_record(home, &rec) {
            errors.push(format!("授权台账更新失败：{}", e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

/// 目的：一条授权没有归属了——按快照还原（产品根内）或精确撤掉自己的 ACE（根外 / 同路径还有别的 SID）。
fn finalize_grant(
    rec: &mut GrantRecord,
    sid_text: &str,
    key: &str,
    recursive: bool,
    errors: &mut Vec<String>,
) {
    let path = PathBuf::from(key);
    let has_other = rec
        .grants
        .iter()
        .any(|g| g.sid != sid_text && g.path == key);
    let snapshot = rec.snapshots.iter().find(|s| s.path == key).cloned();
    if std::fs::symlink_metadata(&path).is_err() {
        // 路径不在了：ACE 跟着没了，销账即可。
        rec.grants.retain(|g| !(g.sid == sid_text && g.path == key));
        if !has_other {
            rec.snapshots.retain(|s| s.path != key);
        }
        return;
    }
    if !has_other {
        if let Some(snap) = snapshot {
            // 根内：最后一个归属走了，按快照整体还原。
            match restore_sd(&path, &snap.bytes) {
                Ok(()) => {
                    rec.grants.retain(|g| !(g.sid == sid_text && g.path == key));
                    rec.snapshots.retain(|s| s.path != key);
                }
                Err(e) => errors.push(e),
            }
            return;
        }
    }
    // 根外（或同路径还有别的 SID）：只精确撤自己这条 ACE。
    match revoke_one_text(sid_text, &path, recursive) {
        Ok(()) => {
            rec.grants.retain(|g| !(g.sid == sid_text && g.path == key));
        }
        Err(e) => errors.push(e),
    }
}

/// 目的：按**会话租约**撤销本进程为该会话写下的全部授权与 profile（会话改配 / 删除时调用）。
/// 约束：只撤"当前进程 + 该租约"的归属；同会话的其它进程、其它会话与工具链 baseline 都不受影响。
pub fn release_by_lease(home: &Path, lease: &str) -> Result<(), String> {
    if lease.trim().is_empty() {
        return Err("按会话释放需要一个租约（空租约不留归属）".to_string());
    }
    let me = current_owner();
    let matches =
        |o: &super::super::super::Owner| o.pid == me.pid && o.start == me.start && o.lease == lease;
    let _lock = lock_ledger(home)?;
    let had_record = record_path(home).exists();
    let mut rec = load_record(home);
    let mut keys: Vec<(String, String)> = rec
        .grants
        .iter()
        .filter(|g| g.owners.iter().any(matches))
        .map(|g| (g.sid.clone(), g.path.clone()))
        .collect();
    keys.sort();
    keys.dedup();
    let mut errors: Vec<String> = Vec::new();
    for (sid_text, key) in &keys {
        let (empty, recursive) = match rec
            .grants
            .iter_mut()
            .find(|g| g.sid == *sid_text && g.path == *key)
        {
            Some(g) => {
                g.owners
                    .retain(|o| !(o.pid == me.pid && o.start == me.start && o.lease == lease));
                (g.owners.is_empty(), g.recursive)
            }
            None => continue,
        };
        if !empty {
            continue;
        }
        finalize_grant(&mut rec, sid_text, key, recursive, &mut errors);
    }
    // profile 与授权同生命周期：这个会话的归属走了就删 profile（删不掉如实记错）。
    let mut names: Vec<String> = rec
        .profiles
        .iter()
        .filter(|p| p.owners.iter().any(matches))
        .map(|p| p.name.clone())
        .collect();
    names.sort();
    names.dedup();
    for name in &names {
        let empty = match rec.profiles.iter_mut().find(|p| p.name == *name) {
            Some(p) => {
                p.owners
                    .retain(|o| !(o.pid == me.pid && o.start == me.start && o.lease == lease));
                p.owners.is_empty()
            }
            None => continue,
        };
        if empty && delete_profile(name) {
            rec.profiles.retain(|p| p.name != *name);
        }
    }
    if had_record {
        if let Err(e) = save_record(home, &rec) {
            errors.push(format!("授权台账更新失败：{}", e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}
