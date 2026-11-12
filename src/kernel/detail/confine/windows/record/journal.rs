//! 目的：授权日志——每条 ACL 改动先记进台账，再动本机权限项。
//! 管：容器 profile、根外授权摘要、根内文件快照的记账，按 (SID, 路径, 权限) 合并同一条，以及授权写后的核对。
//! 不管：ACL 的读写与还原（acl.rs），也不管台账文件本身的锁与落盘（store.rs）。
//! 联动：windows/mod.rs 的 prepare_fence 逐条调用；盘上格式定义在 store.rs。

use super::super::super::Owner;
use super::super::*;
use std::path::Path;
use windows_sys::Win32::Security::PSID;

/// 目的：把容器 profile 名先记进台账落盘（守门进程真正建 profile 之前，名字先有主），并记下归属。
/// 错误：台账落盘失败时返回原因。
pub(crate) fn journal_add_profile(
    home: &Path,
    rec: &mut GrantRecord,
    name: &str,
    owner: Owner,
) -> Result<(), String> {
    if let Some(p) = rec.profiles.iter_mut().find(|p| p.name == name) {
        if p.owners.contains(&owner) {
            return Ok(());
        }
        p.owners.push(owner.clone());
        if let Err(e) = save_record(home, rec) {
            if let Some(p) = rec.profiles.iter_mut().find(|p| p.name == name) {
                p.owners.retain(|o| *o != owner);
            }
            return Err(format!("授权台账落盘失败：{}", e));
        }
        return Ok(());
    }
    rec.profiles.push(ProfileEntry {
        name: name.to_string(),
        at: now_secs(),
        owners: vec![owner],
    });
    if let Err(e) = save_record(home, rec) {
        rec.profiles.retain(|p| p.name != name);
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(())
}

/// 目的：把一条授权先写进台账落盘（根内根外都记），并把当前归属并进它的归属集合。
/// 返回：(是否新建了条目, 是否新增了归属)——回滚时只撤**确实新增过**的那一份归属。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn journal_claim_grant(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
    rights: u32,
    recursive: bool,
    owner: Owner,
) -> Result<(bool, bool), String> {
    let key = path.to_string_lossy().into_owned();
    if let Some(g) = rec
        .grants
        .iter_mut()
        .find(|g| g.sid == sid && g.path == key)
    {
        if g.owners.contains(&owner) {
            return Ok((false, false));
        }
        g.owners.push(owner.clone());
        if let Err(e) = save_record(home, rec) {
            if let Some(g) = rec
                .grants
                .iter_mut()
                .find(|g| g.sid == sid && g.path == key)
            {
                g.owners.retain(|o| *o != owner);
            }
            return Err(format!("授权台账落盘失败：{}", e));
        }
        return Ok((false, true));
    }
    rec.grants.push(GrantEntry {
        sid: sid.to_string(),
        path: key.clone(),
        rights,
        at: now_secs(),
        recursive,
        owners: vec![owner],
    });
    if let Err(e) = save_record(home, rec) {
        rec.grants.retain(|g| !(g.sid == sid && g.path == key));
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok((true, true))
}

/// 目的：从一条授权里撤掉一个归属；归属空了就把条目删掉。返回是否删除了条目。
/// 错误：台账落盘失败时返回原因（此时内存与盘上可能不一致，调用方按失败处置）。
pub(crate) fn journal_release_grant(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
    owner: Owner,
) -> Result<bool, String> {
    let key = path.to_string_lossy().into_owned();
    let Some(g) = rec
        .grants
        .iter_mut()
        .find(|g| g.sid == sid && g.path == key)
    else {
        return Ok(false);
    };
    if !g.owners.contains(&owner) {
        return Ok(false);
    }
    g.owners.retain(|o| *o != owner);
    let empty = g.owners.is_empty();
    if empty {
        rec.grants.retain(|g| !(g.sid == sid && g.path == key));
    }
    if let Err(e) = save_record(home, rec) {
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(empty)
}

/// 目的：把一条根内路径的原始安全描述符先写进台账落盘；返回本次是否新增了该路径的快照。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn journal_add_snapshot(
    home: &Path,
    rec: &mut GrantRecord,
    path: &Path,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    let key = path.to_string_lossy().into_owned();
    if rec.snapshots.iter().any(|s| s.path == key) {
        return Ok(false);
    }
    rec.snapshots.push(SnapshotEntry {
        path: key.clone(),
        at: now_secs(),
        bytes,
    });
    if let Err(e) = save_record(home, rec) {
        rec.snapshots.retain(|s| s.path != key);
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(true)
}

/// 目的：按“先落台账、再写 ACL、写后核对、失败回滚”完成一条授权。
/// 参数：rec 是本次准备的内存台账；sid_text 是 SID 字符串；target 是落点；root 是产品根；owner 是本次归属。
/// 错误：台账落盘、写后核对或回滚失败时返回原因（回滚失败会一并写进错误）。
/// 返回：true = 真改了 ACL；false = ACE 已够用、只补记了归属（不重复改）。
pub(crate) fn grant_one_journaled(
    home: &Path,
    rec: &mut GrantRecord,
    sid: PSID,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
    owner: Owner,
) -> Result<bool, String> {
    let (path, rights, recursive, inherit) = (
        target.path.as_path(),
        target.rights,
        target.recursive,
        target.inherit,
    );
    let key = path.to_string_lossy().into_owned();
    let inside = inside_root(path, root);
    // 根内路径先落原始安全描述符快照（整体还原的依据）；快照不带归属，是否还需要它由该路径上的授权条目派生。
    let snap_created = if inside {
        let bytes = sd_bytes(path).map_err(|e| format!("读原始安全描述符失败：{}", e))?;
        journal_add_snapshot(home, rec, path, bytes)?
    } else {
        false
    };
    let (_, owner_added) =
        journal_claim_grant(home, rec, sid_text, path, rights, recursive, owner.clone())?;
    // ACE 已够用：归属已经补记（同名 agent 的另一个会话靠它保住这条授权），不重复改 DACL。
    if has_ace_for(sid, path, rights) {
        return Ok(false);
    }
    match grant_verified(sid, path, rights, recursive, inherit) {
        Ok(()) => Ok(true),
        Err(e) => {
            // 写失败 = 这一个字节都没改：grant_verified 只在**写成功**后负责还原。
            // 这里不再按“有没有该 SID 的 ACE”去撤销——预存的部分 ACE 会被误当成自己写的。
            let mut journal: Option<String> = None;
            if owner_added {
                if let Err(x) = journal_release_grant(home, rec, sid_text, path, owner) {
                    journal = Some(x);
                }
            }
            if inside && snap_created && !rec.grants.iter().any(|g| g.path == key) {
                rec.snapshots.retain(|s| s.path != key);
                if let Err(x) = save_record(home, rec) {
                    journal = Some(format!("台账更新失败：{}", x));
                }
            }
            Err(combine(e, journal))
        }
    }
}

/// 目的：把主错误与回滚、台账更新两条次生错误拼成一条如实的失败原因。
fn combine(primary: String, journal: Option<String>) -> String {
    let mut out = primary;
    if let Some(j) = journal {
        out.push_str("；台账更新失败：");
        out.push_str(&j);
    }
    out
}
