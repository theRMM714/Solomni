//! 目的：启动对账——把盘上台账与本机实际权限项对一遍，回收该回收的、跳过判不了的。
//! 管：先备份台账，再分授权 / 快照 / profile 三段回收，并给出一份可读报告（跳过、保留、回收与错误）。
//! 不管：单条授权的建立（journal.rs）与单条人工处置（catalog.rs）。
//! 联动：入口是 --fence-reconcile（src/main.rs）；落盘与锁在 store.rs，判活在 liveness.rs。

use super::super::*;
use crate::kernel::detail::confine::ledger::ReconcileReport;
use std::path::{Path, PathBuf};

/// 目的：启动期对账——按台账回收"归属明确已死"的陈旧授权；无法判定的报告后跳过。
/// 约束：不问用户；只回收归属明确已死的条目（活着的不动）；失败保留台账供下次重试。
///   **不做**孤儿清扫（sweep_orphan_aces / sweep_profiles）——那两条无归属，会误伤在跑实例，只归手动 --fence-clean；
///   但会**报告**台账之外的按次 profile（按次调用后数量随调用增长），处置入口仍是手动。
pub fn reconcile(home: &Path) -> ReconcileReport {
    let mut rep = ReconcileReport::default();
    let _lock = match lock_ledger(home) {
        Ok(l) => l,
        Err(e) => {
            rep.errors.push(e);
            return rep;
        }
    };
    let mut rec = load_record(home);
    // 台账外的本程序 profile：只列出来（没有归属证明，自动删会误伤在跑实例）。
    match our_profile_names() {
        Ok(names) => {
            let known: Vec<String> = rec
                .profiles
                .iter()
                .map(|p| p.name.to_ascii_lowercase())
                .collect();
            rep.orphan_profiles = names
                .into_iter()
                .filter(|n| !known.contains(&n.to_ascii_lowercase()))
                .collect();
        }
        Err(e) => rep.errors.push(format!("列举容器 profile 失败：{}", e)),
    }
    if rec.is_empty() {
        rep.note = "没有台账：本程序没在本机写过权限项".to_string();
        return rep;
    }
    // 归属分类：机器级授权一律 Keep（不撤 ACE、不还原其快照，计入 kept）；按次条目按归属集合判。
    let grant_state: Vec<OwnersState> = rec.grants.iter().map(entry_state).collect();
    let profile_state: Vec<OwnersState> = rec
        .profiles
        .iter()
        .map(|p| owners_state(&p.owners))
        .collect();
    // 快照：路径上只要还有一条"非可回收"的授权就保留；否则可回收（含没有任何授权引用的孤儿快照）。
    let snap_reclaim: Vec<bool> = rec
        .snapshots
        .iter()
        .map(|s| {
            !rec.grants
                .iter()
                .zip(&grant_state)
                .any(|(g, st)| g.path == s.path && *st != OwnersState::Reclaim)
        })
        .collect();
    rep.skipped_unjudgeable = grant_state
        .iter()
        .filter(|s| **s == OwnersState::Unjudgeable)
        .count()
        + profile_state
            .iter()
            .filter(|s| **s == OwnersState::Unjudgeable)
            .count();
    rep.kept = grant_state
        .iter()
        .filter(|s| **s != OwnersState::Reclaim)
        .count()
        + snap_reclaim.iter().filter(|b| !**b).count()
        + profile_state
            .iter()
            .filter(|s| **s != OwnersState::Reclaim)
            .count();
    let to_do = grant_state.contains(&OwnersState::Reclaim)
        || profile_state.contains(&OwnersState::Reclaim)
        || snap_reclaim.iter().any(|b| *b);
    if !to_do {
        return rep;
    }
    // 清理前先备份整份台账（审计 + 想重跑/核对的依据）。
    match backup_record(home) {
        Ok(p) => rep.backup = p.map(|x| x.to_string_lossy().into_owned()),
        Err(e) => rep.errors.push(format!("台账备份失败（继续回收）：{}", e)),
    }
    let mut errors: Vec<String> = Vec::new();
    // ① 逐条撤"根外 / 同路径还有保留授权"的授权；根内且是最后一个归属的交给快照整体还原。
    for (i, g) in rec.grants.iter().enumerate() {
        if grant_state[i] != OwnersState::Reclaim {
            continue;
        }
        let path = PathBuf::from(&g.path);
        if std::fs::symlink_metadata(&path).is_err() {
            // 路径不在了：ACE 跟着没了，销账即可。
            rep.reclaimed_grants += 1;
            continue;
        }
        let kept_here = rec
            .grants
            .iter()
            .zip(&grant_state)
            .any(|(o, st)| o.path == g.path && *st != OwnersState::Reclaim);
        let has_snap = rec.snapshots.iter().any(|s| s.path == g.path);
        if has_snap && !kept_here {
            continue; // 交给下面快照整体还原
        }
        match revoke_one_text(&g.sid, &path, g.recursive) {
            Ok(()) => rep.reclaimed_grants += 1,
            Err(e) => errors.push(e),
        }
    }
    // ② 快照整体还原（路径上已无保留授权，或本来就是孤儿快照）。
    for (i, s) in rec.snapshots.iter().enumerate() {
        if !snap_reclaim[i] {
            continue;
        }
        let path = PathBuf::from(&s.path);
        if std::fs::symlink_metadata(&path).is_err() {
            rep.restored_snapshots += 1;
            continue;
        }
        match restore_sd(&path, &s.bytes) {
            Ok(()) => rep.restored_snapshots += 1,
            Err(e) => errors.push(e),
        }
    }
    // ③ 删掉归属明确已死的容器 profile。
    for (i, p) in rec.profiles.iter().enumerate() {
        if profile_state[i] != OwnersState::Reclaim {
            continue;
        }
        if !profile_exists(&p.name) {
            rep.deleted_profiles += 1;
            continue;
        }
        if delete_profile(&p.name) {
            rep.deleted_profiles += 1;
        } else {
            errors.push(format!("删不掉容器 profile：{}", p.name));
        }
    }
    if errors.is_empty() {
        // 全部成功：把可回收的条目从台账里销掉。
        rec.grants = rec
            .grants
            .iter()
            .zip(&grant_state)
            .filter(|(_, st)| **st != OwnersState::Reclaim)
            .map(|(g, _)| g.clone())
            .collect();
        rec.snapshots = rec
            .snapshots
            .iter()
            .zip(&snap_reclaim)
            .filter(|(_, r)| !**r)
            .map(|(s, _)| s.clone())
            .collect();
        rec.profiles = rec
            .profiles
            .iter()
            .zip(&profile_state)
            .filter(|(_, st)| **st != OwnersState::Reclaim)
            .map(|(p, _)| p.clone())
            .collect();
        if let Err(e) = save_record(home, &rec) {
            errors.push(format!("台账更新失败：{}", e));
        } else {
            rec.drop_if_empty(home);
        }
    }
    rep.errors.extend(errors);
    rep
}
