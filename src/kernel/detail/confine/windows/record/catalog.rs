//! 目的：台账的按条视图与按条处置——列清单、还原一条、撤一条、删一个 profile。
//! 管：把台账做成 Ledger/LedgerEntry 清单、单条快照还原、单条授权撤销、单条 profile 删除。
//! 不管：整份台账的收尾（release.rs 的 clean）与启动对账（reconcile.rs）。
//! 联动：按条处置入口与 confine/prep.rs 共用这些函数；ACE 读法在 acl.rs。

use super::super::super::{Ledger, LedgerEntry};
use super::super::*;
use std::path::{Path, PathBuf};

/// 读一个落点现在的样子：显式包 SID 允许 ACE + 如实记下的读不到 / 认不出。
/// 返回：ace_sids 是见到的显式包 SID；notes 是读不到 DACL 或认不出的条数（不静默吞）。
fn acl_facts(path: &Path) -> (Vec<String>, Vec<String>) {
    match acl_scan(path) {
        Ok(scan) => {
            let mut notes = Vec::new();
            if scan.unparsed > 0 {
                notes.push(format!("这个落点上有 {} 条布局认不出的 ACE", scan.unparsed));
            }
            (orphan_package_aces(path).unwrap_or_default(), notes)
        }
        Err(e) => (Vec::new(), vec![e]),
    }
}

/// 目的：列出台账现值——快照（路径 + 时间）、根外授权（SID / 路径 / 权限位）、容器 profile，
///   以及“当前实际 ACE 与台账对不对得上”的差异。
/// 约束：ACE 的读法只有一处（acl_scan / ace_parts / orphan_package_aces），此处不另写一套。
pub fn catalog(home: &Path) -> Ledger {
    // 列清单也要读一致性快照：写侧都在锁里，读侧不持锁可能读到写了一半的文件。
    // 没有台账就不建私有区、不加锁（只读场景不产生副作用）。
    let _lock = if record_path(home).exists() {
        match lock_ledger(home) {
            Ok(l) => Some(l),
            Err(e) => {
                return Ledger {
                    note: format!("台账加锁失败：{}", e),
                    entries: Vec::new(),
                };
            }
        }
    } else {
        None
    };
    let rec = load_record(home);
    if rec.is_empty() {
        return Ledger {
            note: "没有台账：本程序没在本机写过权限项".to_string(),
            entries: Vec::new(),
        };
    }
    let mut entries: Vec<LedgerEntry> = Vec::new();
    for snap in &rec.snapshots {
        let p = PathBuf::from(&snap.path);
        let exists = std::fs::symlink_metadata(&p).is_ok();
        let (ace_sids, notes) = if exists {
            acl_facts(&p)
        } else {
            (Vec::new(), vec!["这个路径现在不在了".to_string()])
        };
        entries.push(LedgerEntry {
            kind: "snapshot",
            sid: String::new(),
            path: snap.path.clone(),
            rights: None,
            at: snap.at,
            present: exists,
            owners: Vec::new(),
            ace_sids,
            notes,
        });
    }
    for grant in &rec.grants {
        let p = PathBuf::from(&grant.path);
        let exists = std::fs::symlink_metadata(&p).is_ok();
        let (ace_sids, mut notes) = if exists {
            acl_facts(&p)
        } else {
            (Vec::new(), vec!["这个路径现在不在了".to_string()])
        };
        // 差异：台账说授过这个 SID，盘上现在还挂着它的 ACE 吗（看**在不在场**，不看权限位够不够）。
        let mut present = false;
        if exists {
            match sid_from_string(&grant.sid) {
                Ok(sid) => {
                    present = has_any_ace_for(sid, &p);
                    free_sid(sid);
                }
                Err(e) => notes.push(e),
            }
            if !present {
                notes.push("台账里记着这条授权，盘上已经没有这个 SID 的允许 ACE".to_string());
            }
        }
        // 机器级授权的口径要说清：它跨会话复用、启动对账不回收，撤它得显式清或按条撤。
        if grant.scope == GrantScope::Machine {
            notes.push(
                "机器级授权：授给本产品能力 SID、跨会话复用；启动对账不回收（要撤用 --fence-clean 或按条撤销）"
                    .to_string(),
            );
        }
        entries.push(LedgerEntry {
            kind: "grant",
            sid: grant.sid.clone(),
            path: grant.path.clone(),
            rights: Some(grant.rights),
            at: grant.at,
            present,
            owners: grant.owners.clone(),
            ace_sids,
            notes,
        });
    }
    for prof in &rec.profiles {
        let present = profile_exists(&prof.name);
        let notes = if present {
            Vec::new()
        } else {
            vec!["这个 profile 现在不在了".to_string()]
        };
        entries.push(LedgerEntry {
            kind: "profile",
            sid: String::new(),
            path: prof.name.clone(),
            rights: None,
            at: prof.at,
            present,
            owners: prof.owners.clone(),
            ace_sids: Vec::new(),
            notes,
        });
    }
    Ledger {
        note: String::new(),
        entries,
    }
}

/// 目的：按路径**只还原一条**快照（其余条目与整份 DACL 不受影响）。
/// 返回：还原成功的一句话（含“台账里已没有这条”这类如实说明）。
/// 错误：台账里没有该路径、路径不在了、写回被拒，都如实返回；失败时台账**不改**（供重试）。
pub fn restore_one(home: &Path, path: &Path) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let mut rec = load_record(home);
    let key = path.to_string_lossy().into_owned();
    let Some(idx) = rec.snapshots.iter().position(|s| s.path == key) else {
        return Err(format!("台账里没有这个路径的快照：{}", key));
    };
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("这个路径现在不在了：{}", key));
    }
    restore_sd(path, &rec.snapshots[idx].bytes)
        .map_err(|e| format!("还原未完成（{}）：{}", key, e))?;
    rec.snapshots.remove(idx);
    // 整体还原会连该路径上所有我们写下的 ACE 一起撤掉：那些授权条目也一并销账。
    rec.grants.retain(|g| g.path != key);
    if let Err(e) = save_record(home, &rec) {
        return Err(format!("已还原该条，但台账更新失败：{}", e));
    }
    rec.drop_if_empty(home);
    Ok(format!("已按台账还原这一条快照的原始权限：{}", key))
}

/// 目的：按 **SID + 路径**只撤一条授权（其余条目不受影响）；**不在台账里也照撤**。
/// 返回：撤权结果的一句话（是否在台账里如实标注）；盘上本来就没有该 SID 的 ACE 时也如实说。
/// 错误：路径不在了、SID 不合法、写撤权后的 DACL 被拒，都如实返回；失败时台账不改动。
pub fn revoke_grant(home: &Path, sid_text: &str, path: &Path) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let key = path.to_string_lossy().into_owned();
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("这个路径现在不在了：{}", key));
    }
    let sid = sid_from_string(sid_text)?;
    let present = has_any_ace_for(sid, path);
    let outcome = if present {
        revoke_one(sid, path, true)
    } else {
        Ok(())
    };
    free_sid(sid);
    outcome.map_err(|e| format!("撤销未完成（{}，{}）：{}", sid_text, key, e))?;
    // 台账里有这一条就按条删掉；没有就如实说它不在台账里（照样撤）。
    let mut rec = load_record(home);
    let had = rec
        .grants
        .iter()
        .any(|g| g.sid == sid_text && g.path == key);
    if had {
        rec.grants.retain(|g| !(g.sid == sid_text && g.path == key));
        // 该路径上再没有任何授权引用时，快照也一并销掉（没有可管理的 ACE 了）。
        if !rec.grants.iter().any(|g| g.path == key) {
            rec.snapshots.retain(|s| s.path != key);
        }
        if let Err(e) = save_record(home, &rec) {
            return Err(format!("已撤掉该条 ACE，但台账更新失败：{}", e));
        }
        rec.drop_if_empty(home);
    }
    Ok(match (present, had) {
        (true, true) => format!("已按台账撤销这一条授权：{} → {}", sid_text, key),
        (true, false) => format!(
            "已撤销这一条 ACE（它**不在台账里**——台账外残留）：{} → {}",
            sid_text, key
        ),
        (false, true) => format!(
            "盘上已经没有这个 SID 的允许 ACE；台账里那一条已删：{} → {}",
            sid_text, key
        ),
        (false, false) => format!(
            "盘上本来就没有这个 SID 的允许 ACE，台账里也没有这一条：{} → {}",
            sid_text, key
        ),
    })
}

/// 目的：按名**只删一个**容器 profile（连该容器的存储一起删）。
/// 返回：删除结果的一句话（是否在台账里如实标注）；盘上本来就没有这个 profile 时也如实说。
/// 错误：删除被拒时如实返回；失败时台账不改动。
pub fn remove_profile_one(home: &Path, name: &str) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let existed = profile_exists(name);
    if existed && !delete_profile(name) {
        return Err(format!("删不掉这个容器 profile：{}", name));
    }
    let mut rec = load_record(home);
    let had = rec.profiles.iter().any(|p| p.name == name);
    if had {
        rec.profiles.retain(|p| p.name != name);
        if let Err(e) = save_record(home, &rec) {
            return Err(format!("已删掉该 profile，但台账更新失败：{}", e));
        }
        rec.drop_if_empty(home);
    }
    Ok(match (existed, had) {
        (true, true) => format!("已按台账删掉这个容器 profile：{}", name),
        (true, false) => format!("已删掉这个容器 profile（它**不在台账里**）：{}", name),
        (false, true) => format!("这个 profile 已经不在了；台账里那一条已删：{}", name),
        (false, false) => format!("这个 profile 本来就不在，台账里也没有这一条：{}", name),
    })
}
