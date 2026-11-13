//! 目的：授权日志——每条 ACL 改动先记进台账，再动本机权限项。
//! 管：容器 profile、根外授权摘要、根内文件快照的记账，按 (SID, 路径, 权限) 合并同一条，以及授权写后的核对。
//! 不管：ACL 的读写与还原（acl.rs），也不管台账文件本身的锁与落盘（store.rs）。
//! 联动：windows.rs 的 prepare_fence 逐条调用；台账事务只罩「读—改—写」（store.rs 的 with_ledger），
//!   慢的整树 ACL 写在锁外——同进程并发调用不因 ACL 写互相排队。

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
        scope: GrantScope::Call,
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

/// 目的：一条「按次」授权在写 ACL 前先落台账的返回值——回滚时只撤本次确实新增的那一份。
#[derive(Debug, Clone, Copy)]
pub(crate) struct OneClaim {
    /// 目的：本次是否新增了归属（回滚只撤这一份）。
    pub(crate) owner_added: bool,
    /// 目的：本次是否新增了根内快照。
    pub(crate) snap_created: bool,
}

/// 目的：一条「机器级」授权在写 ACL 前先落台账的返回值。
#[derive(Debug, Clone, Copy)]
pub(crate) struct MachineClaim {
    /// 目的：本次是否新建了台账条目（回滚只撤新建的）。
    pub(crate) created: bool,
    /// 目的：本次是否新增了根内快照。
    pub(crate) snap_created: bool,
}

/// 目的：按次授权的「先落台账」阶段：快照 + 归属，一次锁内的台账事务（毫秒级），返回后锁即释放。
/// 约束：写 ACL 由调用方在锁外做——慢的整树继承重算不能占着台账锁。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn claim_one(
    home: &Path,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
    owner: Owner,
) -> Result<OneClaim, String> {
    with_ledger(home, |rec| {
        let snap_created = if inside_root(&target.path, root) {
            let bytes =
                sd_bytes(&target.path).map_err(|e| format!("读原始安全描述符失败：{}", e))?;
            journal_add_snapshot(home, rec, &target.path, bytes)?
        } else {
            false
        };
        let (_, owner_added) = journal_claim_grant(
            home,
            rec,
            sid_text,
            &target.path,
            target.rights,
            target.recursive,
            owner,
        )?;
        Ok(OneClaim {
            owner_added,
            snap_created,
        })
    })
}

/// 目的：按次授权写 ACL 失败后的回滚：撤本次新增的归属 / 快照（锁内一次台账事务）。
/// 约束：只在 grant_verified 明确失败（一个字节都没改）时调用；不按 ACE 反推。
pub(crate) fn abort_one(
    home: &Path,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
    owner: Owner,
    claim: OneClaim,
) -> Result<(), String> {
    let inside = inside_root(&target.path, root);
    let key = target.path.to_string_lossy().into_owned();
    with_ledger(home, |rec| {
        if claim.owner_added {
            journal_release_grant(home, rec, sid_text, &target.path, owner)?;
        }
        if inside && claim.snap_created && !rec.grants.iter().any(|g| g.path == key) {
            rec.snapshots.retain(|s| s.path != key);
            save_record(home, rec)?;
        }
        Ok(())
    })
}

/// 目的：按「先落台账、再写 ACL、写后核对、失败回滚」完成一条按次授权。
/// 参数：sid_text 是 SID 字符串；target 是落点；root 是产品根；owner 是本次归属。
/// 错误：台账落盘、写后核对或回滚失败时返回原因（回滚失败会一并写进错误）。
/// 返回：true = 真改了 ACL；false = ACE 已够用、只补记了归属（不重复改）。
/// 约束：台账事务（claim / abort）各自加锁；中间的 ACL 读写不持锁（慢的整树重算在锁外）。
pub(crate) fn grant_one_journaled(
    home: &Path,
    sid: PSID,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
    owner: Owner,
) -> Result<bool, String> {
    let claim = claim_one(home, sid_text, target, root, owner.clone())?;
    // ACE 已够用：归属已经补记（同名 agent 的另一个会话靠它保住这条授权），不重复改 DACL。
    if has_ace_for(sid, &target.path, target.rights) {
        return Ok(false);
    }
    match grant_verified(
        sid,
        &target.path,
        target.rights,
        target.recursive,
        target.inherit,
    ) {
        Ok(()) => Ok(true),
        Err(e) => {
            // 写失败 = 这一个字节都没改：grant_verified 只在**写成功**后负责还原。
            // 这里不再按「有没有该 SID 的 ACE」去撤销——预存的部分 ACE 会被误当成自己写的。
            let journal = abort_one(home, sid_text, target, root, owner, claim).err();
            Err(combine(e, journal))
        }
    }
}

/// 目的：把一条**机器级**授权写进台账落盘（授给所有应用程序包，**不记任何进程归属**）。
/// 返回：true = 本次新建了条目；false = 条目已在台账里（顺带把口径归正为机器级、清掉旧归属）。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
/// 约束：不能复用 journal_claim_grant 塞一个假归属——机器级条目的 owners 必须真的是空。
pub(crate) fn journal_claim_machine(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
    rights: u32,
    recursive: bool,
) -> Result<bool, String> {
    let key = path.to_string_lossy().into_owned();
    if let Some(g) = rec
        .grants
        .iter_mut()
        .find(|g| g.sid == sid && g.path == key)
    {
        let same = g.scope == GrantScope::Machine
            && g.owners.is_empty()
            && g.rights == rights
            && g.recursive == recursive;
        if same {
            return Ok(false);
        }
        // 已在台账里（旧条目可能是按次记的）：口径归正为机器级，归属清空。
        let before = g.clone();
        g.scope = GrantScope::Machine;
        g.owners.clear();
        g.rights = rights;
        g.recursive = recursive;
        if let Err(e) = save_record(home, rec) {
            // 落盘失败：内存回到调用前的形状，调用方之后的回滚才不会按错口径处理。
            if let Some(g) = rec
                .grants
                .iter_mut()
                .find(|g| g.sid == sid && g.path == key)
            {
                *g = before;
            }
            return Err(format!("授权台账落盘失败：{}", e));
        }
        return Ok(false);
    }
    rec.grants.push(GrantEntry {
        sid: sid.to_string(),
        path: key.clone(),
        rights,
        at: now_secs(),
        recursive,
        owners: Vec::new(),
        scope: GrantScope::Machine,
    });
    if let Err(e) = save_record(home, rec) {
        rec.grants.retain(|g| !(g.sid == sid && g.path == key));
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(true)
}

/// 目的：删掉一条**本次新建**的机器级授权条目（写 ACL 失败时回滚用）；已存在的条目不动。
/// 错误：台账落盘失败时返回原因。
pub(crate) fn journal_drop_machine(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
) -> Result<(), String> {
    let key = path.to_string_lossy().into_owned();
    if !rec
        .grants
        .iter()
        .any(|g| g.sid == sid && g.path == key && g.scope == GrantScope::Machine)
    {
        return Ok(());
    }
    rec.grants.retain(|g| !(g.sid == sid && g.path == key));
    save_record(home, rec).map_err(|e| format!("授权台账落盘失败：{}", e))
}

/// 目的：机器级授权的「先落台账」阶段：快照 + 机器级条目，一次锁内的台账事务（毫秒级）。
/// 约束：写 ACL 由调用方在锁外做——慢的整树继承重算不能占着台账锁。
pub(crate) fn claim_machine(
    home: &Path,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
) -> Result<MachineClaim, String> {
    with_ledger(home, |rec| {
        let snap_created = if inside_root(&target.path, root) {
            let bytes =
                sd_bytes(&target.path).map_err(|e| format!("读原始安全描述符失败：{}", e))?;
            journal_add_snapshot(home, rec, &target.path, bytes)?
        } else {
            false
        };
        let created = journal_claim_machine(
            home,
            rec,
            sid_text,
            &target.path,
            target.rights,
            target.recursive,
        )?;
        Ok(MachineClaim {
            created,
            snap_created,
        })
    })
}

/// 目的：机器级授权写 ACL 失败后的回滚：撤本次新建的条目 / 快照（锁内一次台账事务）。
/// 约束：只在 grant_verified 明确失败（一个字节都没改）时调用；已有条目与它的快照不动。
pub(crate) fn abort_machine(
    home: &Path,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
    claim: MachineClaim,
) -> Result<(), String> {
    let inside = inside_root(&target.path, root);
    let key = target.path.to_string_lossy().into_owned();
    with_ledger(home, |rec| {
        if claim.created {
            journal_drop_machine(home, rec, sid_text, &target.path)?;
        }
        if inside && claim.snap_created && !rec.grants.iter().any(|g| g.path == key) {
            rec.snapshots.retain(|s| s.path != key);
            save_record(home, rec)?;
        }
        Ok(())
    })
}

/// 目的：按「先落台账、再写 ACL、写后核对、失败回滚」完成一条**机器级**授权（授给所有应用程序包）。
/// 参数：sid/sid_text 是基线 SID；target 是落点（祖先链 / 可信基座）；root 是产品根。
/// 错误：台账落盘、写后核对或回滚失败时返回原因。
/// 返回：true = 真改了 ACL；false = ACE 已够用、只补记了台账（不重复改）。
/// 约束：与 grant_one_journaled 同一套快照与核对语义，区别只在归属：机器级不记 owner，回滚只撤本次新建的条目。
pub(crate) fn grant_machine_journaled(
    home: &Path,
    sid: PSID,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
) -> Result<bool, String> {
    let claim = claim_machine(home, sid_text, target, root)?;
    // ACE 已够用（后续会话就走到这里）：不重复改 DACL，台账已记着这条机器级授权。
    if has_ace_for(sid, &target.path, target.rights) {
        return Ok(false);
    }
    match grant_verified(
        sid,
        &target.path,
        target.rights,
        target.recursive,
        target.inherit,
    ) {
        Ok(()) => Ok(true),
        Err(e) => {
            // 写失败 = 这一个字节都没改：这里只撤本次新建的台账条目 / 快照，不按 ACE 反推。
            let journal = abort_machine(home, sid_text, target, root, claim).err();
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
