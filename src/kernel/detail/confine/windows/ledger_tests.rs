//! 目的：Windows 围栏台账的真机契约——快照先落盘、写后核对、按条处置、启动对账与孤儿回收。
//! 管：需要真改 ACL 的台账一族用例；本机做不了往返预检就如实跳过。
//! 不管：容器身份与容器内往返（见 `tests.rs`）、非 Windows 平台（见 tests/linux、tests/macos）。
//! 联动：机制在 confine/windows.rs 与 ledger.rs；夹具复用 `tests` 模块的探针助手。

use super::super::Owner;
use super::tests::{acl_round_trip, discard, expect_granted};
use super::*;

/// 【台账契约】写前先落盘：快照与授权摘要都要能在 ACL 改动前读回；收尾按快照整体还原。
#[test]
fn journal_records_snapshot_and_grant_before_touching_acl() {
    let base = std::env::temp_dir().join(format!("solomni-journal-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "journal") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：台账探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let bytes = sd_bytes(&target).expect("读原始安全描述符");
    let mut rec = load_record(&home);
    assert!(
        journal_add_snapshot(&home, &mut rec, &target, bytes.clone()).expect("台账先落盘"),
        "第一次写该路径的快照应当新增"
    );
    let reread = load_record(&home);
    assert!(
        reread
            .snapshots
            .iter()
            .any(|s| Path::new(&s.path) == target && s.bytes == bytes),
        "动 ACL 之前台账里就要有原始安全描述符"
    );
    let spec = FenceSpec {
        agent: "probe-journal".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let rec = load_record(&home);
    assert!(
        rec.snapshots.iter().any(|s| Path::new(&s.path) == target),
        "收尾还原要用的快照必须在场"
    );
    clean(&home).expect("回收应当成功");
    discard(&base);
}

/// 【写后核对】写下前先记 ACE 集合；写入并还原后，原有 ACE 集合必须与之前一致。
#[test]
fn write_then_restore_keeps_the_original_ace_set() {
    let base = std::env::temp_dir().join(format!("solomni-multiset-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "multiset") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：ACE 集合探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let before = acl_entries(&target).expect("读写入前 ACE 集合");
    let bytes = sd_bytes(&target).expect("存原始安全描述符");
    let spec = FenceSpec {
        agent: "probe-multiset".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    grant_verified(sid, &target, RIGHTS_RO, false, false).expect("写入并核对应当成功");
    assert!(
        has_ace_for(sid, &target, RIGHTS_RO),
        "写入后我们的 ACE 要在场"
    );
    free_sid(sid);
    restore_sd(&target, &bytes).expect("还原原始安全描述符");
    let after = acl_entries(&target).expect("读还原后 ACE 集合");
    // 集合语义：还原可能把重复 ACE 收敛成一份，出现次数变少不算不一致；不同三元组必须一致。
    assert!(
        lost_entries(&before, &after).is_empty() && lost_entries(&after, &before).is_empty(),
        "写入并还原后原有 ACE 集合必须一致"
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(
        !has_any_ace_for(sid, &target),
        "还原后不该再有该容器 SID 的 ACE"
    );
    free_sid(sid);
    discard(&base);
}

/// 【缺落点跳过】路径不存在的落点只跳过、不判整次失败：存在的照常授权，台账里不留不存在的那条。
#[test]
fn missing_grant_target_is_skipped_and_not_journaled() {
    let base = std::env::temp_dir().join(format!("solomni-skip-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "skip") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：缺落点探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let absent = base.join("missing");
    let spec = FenceSpec {
        agent: "probe-skip".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone(), absent.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    let outcome = prepare_fence(&spec, "cmd", &home);
    expect_granted(&outcome, "不存在的落点应跳过、不判整次失败");
    let rec = load_record(&home);
    assert!(
        !rec.snapshots.iter().any(|s| Path::new(&s.path) == absent),
        "跳过的落点不得留快照"
    );
    assert!(
        !rec.grants.iter().any(|g| Path::new(&g.path) == absent),
        "跳过的落点不得留 ACE 摘要"
    );
    assert!(
        rec.snapshots.iter().any(|s| Path::new(&s.path) == target),
        "存在的落点要照常授权"
    );
    clean(&home).expect("回收应当成功");
    discard(&base);
}

/// 【孤儿回收】没有台账、也没有 profile 时，按显式包 SID 也要能连树撤掉残留。
#[test]
fn sweep_reclaims_orphan_package_ace_without_a_ledger() {
    let base = std::env::temp_dir().join(format!("solomni-sweep-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "sweep") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：孤儿回收探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let sid = container_sid("Solomni.Agent.SweepOrphanProbe").expect("派生容器 SID");
    grant_one(sid, &target, RIGHTS_RW, false, false).expect("写下孤儿 ACE");
    assert!(has_any_ace_for(sid, &target), "孤儿 ACE 要在场");
    let swept = sweep_orphan_aces(&base).expect("孤儿清扫应当成功");
    assert!(swept >= 1, "至少在 target 处命中一次");
    assert!(
        !has_any_ace_for(sid, &target),
        "清扫后不该再有该包 SID 的显式 ACE"
    );
    free_sid(sid);
    discard(&base);
}

/// 【先落盘】台账落不了盘时绝不动 ACL：写被台账门禁挡住，目标目录不留任何我们的 ACE。
#[test]
fn journal_failure_blocks_the_acl_write() {
    let base = std::env::temp_dir().join(format!("solomni-journal-gate-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "journal-gate") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：先落盘探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    // home 用一个普通文件占位：save_record 的 create_dir_all 必然失败，台账落不了盘。
    let home = base.join("home-as-file");
    std::fs::write(&home, b"not a dir").expect("写占位文件");
    let spec = FenceSpec {
        agent: "probe-journal-gate".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let outcome = prepare_fence(&spec, "cmd", &home);
    // 台账落不了盘 = **必要**的一环走不动：如实进结论（调用方据此问用户或拒绝，不许降级）。
    assert_eq!(
        outcome.blocked.as_ref().map(|b| b.part),
        Some(FencePart::Ledger),
        "台账落不了盘必须如实报成必要落点授不上：{:?}",
        outcome.blocked
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let wrote = has_ace_for(sid, &target, RIGHTS_RW);
    free_sid(sid);
    assert!(!wrote, "台账没落盘就不许写 ACL");
    discard(&base);
}

/// 【按条处置】清单看得见每一条的现状；按路径只还原一条、按 SID + 路径只撤一条，
/// 其余条目与整份 DACL 不受影响；台账外的根外残留也能按 SID + 路径撤掉并如实标注它不在台账里。
#[test]
fn ledger_catalog_and_per_item_disposal_keep_the_rest_untouched() {
    // 独立临时目录名：另一个用例（第 280 行起）也以 solomni-grant-probe-<pid> 为根，
    // 并行跑会共享同一份台账，必须错开（否则台账条数被对方写脏）。
    let base = std::env::temp_dir().join(format!("solomni-grant-catalog-{}", std::process::id()));
    let inside = base.join("inside");
    let outside =
        std::env::temp_dir().join(format!("solomni-grant-outside-{}", std::process::id()));
    std::fs::create_dir_all(&inside).expect("建根内目录");
    std::fs::create_dir_all(&outside).expect("建根外目录");
    if !acl_round_trip(&base, "grant") || !acl_round_trip(&outside, "grant-outside") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：按条处置探针跳过（不静默当作通过）"
        );
        discard(&base);
        discard(&outside);
        return;
    }
    let home = base.join(".home");
    let spec = FenceSpec {
        agent: "probe-grant".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![inside.clone()],
        cwd: inside.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let sid_text = sid_to_string(sid);
    let owner = current_owner();
    // 一条根内授权（走快照）+ 一条根外授权（走摘要）：都经“先落台账、再写 ACL、写后核对”那条路。
    let inside_target = GrantTarget {
        path: inside.clone(),
        rights: RIGHTS_RW,
        recursive: false,
        inherit: false,
        part: FencePart::DataBoundary,
    };
    grant_one_journaled(&home, sid, &sid_text, &inside_target, &base, owner.clone())
        .expect("根内授权应当成功");
    let outside_target = GrantTarget {
        path: outside.clone(),
        rights: RIGHTS_RO,
        recursive: false,
        inherit: false,
        part: FencePart::Interpreter,
    };
    grant_one_journaled(&home, sid, &sid_text, &outside_target, &base, owner.clone())
        .expect("根外授权应当成功");
    free_sid(sid);
    // 清单：两条都看得见，都如实标成“现在还在”，且与盘上的 ACE 对得上。
    let view = catalog(&home);
    assert_eq!(
        view.entries.len(),
        3,
        "台账里应当有三条（根内快照 + 根内授权 + 根外授权）：{:?}",
        view.entries
    );
    assert!(
        view.entries.iter().all(|e| e.present),
        "两条授权都还在盘上：{:?}",
        view.entries
    );
    assert!(
        view.entries
            .iter()
            .any(|e| e.kind == "snapshot" && Path::new(&e.path) == inside),
        "根内那条要如实标成快照：{:?}",
        view.entries
    );
    assert!(
        view.entries
            .iter()
            .any(|e| e.kind == "grant" && Path::new(&e.path) == outside),
        "根外那条要如实标成授权摘要：{:?}",
        view.entries
    );
    // 按路径只还原根内那一条：它自己的容器 ACE 消失，根外那条与它的整份 DACL 不受影响。
    let before = acl_entries(&outside).expect("读根外 ACE 集合");
    let said = restore_one(&home, &inside).expect("按路径还原应当成功");
    assert!(
        said.contains(&inside.to_string_lossy().into_owned()),
        "{}",
        said
    );
    let sid = container_sid(&container_name(&spec)).expect("再派生容器 SID");
    assert!(
        !has_any_ace_for(sid, &inside),
        "还原后根内那条不该再有该容器 SID 的 ACE"
    );
    assert!(has_any_ace_for(sid, &outside), "其余条目不受影响");
    free_sid(sid);
    assert_eq!(
        acl_entries(&outside).expect("读根外 ACE 集合"),
        before,
        "还原一条不得改动其余条目的 DACL"
    );
    assert_eq!(catalog(&home).entries.len(), 1, "还原过的那条已从台账销掉");
    // 按 SID + 路径撤根外那一条，只动它。
    let said = revoke_grant(&home, &sid_text, &outside).expect("按条撤销应当成功");
    assert!(said.contains("按台账"), "{}", said);
    let sid = container_sid(&container_name(&spec)).expect("再派生容器 SID");
    assert!(
        !has_any_ace_for(sid, &outside),
        "撤过的落点不该再有该 SID 的 ACE"
    );
    free_sid(sid);
    assert!(load_record(&home).is_empty(), "两条都处置完，台账应当清空");
    // 台账外残留：直接写一条 ACE、不经台账，再按 SID + 路径撤掉——如实标注它不在台账里。
    let orphan = container_sid("Solomni.Agent.GrantOrphanProbe").expect("派生孤儿容器 SID");
    let orphan_text = sid_to_string(orphan);
    grant_verified(orphan, &outside, RIGHTS_RO, false, false).expect("写下台账外 ACE");
    let said = revoke_grant(&home, &orphan_text, &outside).expect("台账外残留也要撤得掉");
    assert!(
        said.contains("不在台账里"),
        "要如实标注它不在台账里：{}",
        said
    );
    assert!(!has_any_ace_for(orphan, &outside), "台账外 ACE 应当被撤掉");
    free_sid(orphan);
    // 台账里没有的快照：按条还原如实拒绝（当次调用什么都没做）。
    assert!(
        restore_one(&home, &inside).is_err(),
        "台账里没有这条快照就如实拒绝"
    );
    discard(&base);
    discard(&outside);
}

/// 【启动对账·纯逻辑】归属集合的结论：有活归属则留、全死则回收、缺归属或判不了则跳过。
/// PID 复用（同一 pid、创建时刻对不上）必须判死——否则旧进程的残留永远回收不掉。
#[test]
fn owner_state_distinguishes_live_dead_and_unjudgeable() {
    let me = current_owner();
    if me.start == 0 {
        eprintln!("[探针] 本机读不到进程创建时刻：归属判定用例跳过（不静默当作通过）");
        return;
    }
    let dead = Owner {
        pid: 0x7fff_fff0,
        start: 1,
        lease: String::new(),
    };
    assert_eq!(
        owners_state(std::slice::from_ref(&me)),
        OwnersState::Keep,
        "有活归属要留"
    );
    assert_eq!(
        owners_state(std::slice::from_ref(&dead)),
        OwnersState::Reclaim,
        "全死可回收"
    );
    assert_eq!(
        owners_state(&[me.clone(), dead.clone()]),
        OwnersState::Keep,
        "一活一死不回收"
    );
    assert_eq!(owners_state(&[]), OwnersState::Unjudgeable, "缺归属要跳过");
    assert_eq!(
        owners_state(&[Owner {
            pid: me.pid,
            start: me.start.wrapping_add(1),
            lease: String::new()
        }]),
        OwnersState::Reclaim,
        "PID 被复用（创建时刻对不上）要判死"
    );
}

/// 【启动对账】归属明确已死的陈旧授权必须被回收（ACE 消失、台账销账）；活归属一个字都不动。
#[test]
fn reconcile_reclaims_dead_owners_and_keeps_live_ones() {
    let base = std::env::temp_dir().join(format!("solomni-reconcile-{}", std::process::id()));
    let live = base.join("live");
    let stale = base.join("stale");
    std::fs::create_dir_all(&live).expect("建活落点");
    std::fs::create_dir_all(&stale).expect("建陈旧落点");
    if !acl_round_trip(&base, "reconcile") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：启动对账探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec_live = FenceSpec {
        agent: "probe-reconcile-live".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![live.clone()],
        cwd: live.clone(),
        ro: Vec::new(),
        net: false,
    };
    let spec_stale = FenceSpec {
        agent: "probe-reconcile-stale".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![stale.clone()],
        cwd: stale.clone(),
        ro: Vec::new(),
        net: false,
    };
    expect_granted(&prepare_fence(&spec_live, "cmd", &home), "活授权应当成功");
    expect_granted(
        &prepare_fence(&spec_stale, "cmd", &home),
        "陈旧授权应当成功",
    );
    let stale_sid = container_sid(&container_name(&spec_stale)).expect("派生陈旧 SID");
    let stale_text = sid_to_string(stale_sid);
    free_sid(stale_sid);
    // 把陈旧那条的归属记为一个绝不存在的进程。
    let dead = Owner {
        pid: 0x7fff_fff0,
        start: 1,
        lease: "dead-session".to_string(),
    };
    {
        let mut rec = load_record(&home);
        for g in rec.grants.iter_mut() {
            if g.sid == stale_text {
                g.owners = vec![dead.clone()];
            }
        }
        save_record(&home, &rec).expect("写回台账");
    }
    let rep = reconcile(&home);
    assert!(rep.errors.is_empty(), "对账不该报错：{:?}", rep.errors);
    assert!(
        rep.reclaimed_grants >= 1 || rep.restored_snapshots >= 1,
        "陈旧授权至少要回收一条：{:?}",
        rep
    );
    let sid = container_sid(&container_name(&spec_stale)).expect("再派生陈旧 SID");
    assert!(
        !has_any_ace_for(sid, &stale),
        "陈旧授权回收后不该再有该容器 SID 的 ACE"
    );
    free_sid(sid);
    let sid = container_sid(&container_name(&spec_live)).expect("再派生活 SID");
    assert!(has_any_ace_for(sid, &live), "活归属的授权不许被回收");
    free_sid(sid);
    clean(&home).expect("收尾回收应当成功");
    discard(&base);
}

/// 【启动对账】台账外的按次 profile：只报告，不自动删（没有归属证明，删了会误伤在跑实例）。
#[test]
fn reconcile_reports_ledger_less_profiles_without_deleting_them() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：reconcile_reports_ledger_less_profiles_without_deleting_them 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    let base =
        std::env::temp_dir().join(format!("solomni-reconcile-orphan-{}", std::process::id()));
    let home = base.join(".home");
    std::fs::create_dir_all(&home).expect("建台账目录");
    let spec = FenceSpec {
        agent: "probe-reconcile-orphan".to_string(),
        lease: "call-orphan".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: Vec::new(),
        cwd: PathBuf::new(),
        ro: Vec::new(),
        net: false,
    };
    let name = container_name(&spec);
    // 只建 profile、不记台账（模拟台账被删 / 非台账路径留下的残留）。
    assert!(ensure_profile(&name).is_ok(), "建探针 profile");
    let rep = reconcile(&home);
    assert!(
        rep.orphan_profiles
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&name)),
        "台账外的 profile 要列进报告：{:?}",
        rep.orphan_profiles
    );
    assert!(
        rep.summary().contains("台账外还有"),
        "摘要要说明现象与处置入口：{}",
        rep.summary()
    );
    assert!(profile_exists(&name), "只报告，不自动删");
    // 收尾：探针自己删，不留痕。
    assert!(delete_profile(&name), "探针 profile 应当删得掉");
    discard(&base);
}

/// 【启动对账】缺归属/判不了的条目：如实报告并跳过，一条都不回收。
#[test]
fn reconcile_skips_entries_without_ownership() {
    let base = std::env::temp_dir().join(format!("solomni-reconcile-skip-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "reconcile-skip") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：对账跳过探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec = FenceSpec {
        agent: "probe-reconcile-skip".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    // 清空全部归属（模拟归属读不出来 / 台账丢失）：对账必须报告并跳过，不许回收。
    {
        let mut rec = load_record(&home);
        for g in rec.grants.iter_mut() {
            g.owners.clear();
        }
        for p in rec.profiles.iter_mut() {
            p.owners.clear();
        }
        save_record(&home, &rec).expect("写回台账");
    }
    let rep = reconcile(&home);
    assert!(rep.errors.is_empty(), "对账不该报错：{:?}", rep.errors);
    assert_eq!(rep.reclaimed_grants, 0, "缺归属不得回收");
    assert_eq!(rep.restored_snapshots, 0, "缺归属不得还原快照");
    assert!(
        rep.skipped_unjudgeable >= 1,
        "缺归属要如实报告跳过：{:?}",
        rep
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(has_any_ace_for(sid, &target), "无法判定时不许动 ACE");
    free_sid(sid);
    clean(&home).expect("收尾回收应当成功");
    discard(&base);
}

/// 【机器级·纯逻辑】祖先链授权形状：只有 Parent + 只读属性 + 非递归 + 非继承 才算机器级，其它一律按次。
#[test]
fn machine_target_is_only_the_ancestor_chain_shape() {
    let machine = GrantTarget {
        path: PathBuf::from("C:\\work"),
        rights: RIGHTS_STAT,
        recursive: false,
        inherit: false,
        part: FencePart::Parent,
    };
    assert!(is_machine_target(&machine), "祖先链只读属性是机器级");
    assert!(
        !is_machine_target(&GrantTarget {
            recursive: true,
            ..machine.clone()
        }),
        "递归落点不是祖先链形状"
    );
    assert!(
        !is_machine_target(&GrantTarget {
            inherit: true,
            ..machine.clone()
        }),
        "继承落点不是祖先链形状"
    );
    assert!(
        !is_machine_target(&GrantTarget {
            rights: RIGHTS_RO,
            ..machine.clone()
        }),
        "只读内容不是机器级"
    );
    assert!(
        !is_machine_target(&GrantTarget {
            part: FencePart::Cwd,
            ..machine
        }),
        "非 Parent 落点不是机器级"
    );
}

/// 【机器级·台账】授给所有应用程序包的祖先链授权：不记归属、对账一律保留（不撤 ACE、不还原快照）。
#[test]
fn machine_grant_has_no_owners_and_survives_reconcile() {
    let base = std::env::temp_dir().join(format!("solomni-machine-grant-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "machine") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：机器级台账探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let base_sid = match baseline_sid() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[探针] 取不到「所有应用程序包」基线 SID：机器级台账探针跳过（{}）",
                e
            );
            discard(&base);
            return;
        }
    };
    let base_text = sid_to_string(base_sid);
    let grant = GrantTarget {
        path: target.clone(),
        rights: RIGHTS_STAT,
        recursive: false,
        inherit: false,
        part: FencePart::Parent,
    };
    grant_machine_journaled(&home, base_sid, &base_text, &grant, &base)
        .expect("机器级授权应当先落台账、再写并核对");
    let key = target.to_string_lossy().into_owned();
    {
        let rec = load_record(&home);
        let entry = rec
            .grants
            .iter()
            .find(|g| g.sid == base_text && g.path == key)
            .expect("台账里要有机器级条目");
        assert_eq!(entry.scope, GrantScope::Machine, "祖先链授权要记成机器级");
        assert!(entry.owners.is_empty(), "机器级条目不记任何进程归属");
        assert_eq!(entry_state(entry), OwnersState::Keep, "机器级一律保留");
    }
    // 模拟旧数据/误记归属：即便归属被塞过且那些进程已死，也按口径保留（不靠"空归属"的副作用）。
    {
        let mut rec = load_record(&home);
        for g in rec.grants.iter_mut() {
            if g.scope == GrantScope::Machine {
                g.owners = vec![Owner {
                    pid: 0x7fff_fff0,
                    start: 1,
                    lease: "dead-session".to_string(),
                }];
            }
        }
        save_record(&home, &rec).expect("写回台账");
    }
    let rep = reconcile(&home);
    assert!(rep.errors.is_empty(), "对账不该报错：{:?}", rep.errors);
    assert_eq!(rep.reclaimed_grants, 0, "机器级授权不得回收");
    assert_eq!(rep.restored_snapshots, 0, "机器级授权的快照不得还原");
    assert_eq!(
        rep.skipped_unjudgeable, 0,
        "机器级按口径判 Keep，不能落进缺归属/判不了"
    );
    assert!(rep.kept >= 1, "机器级授权要计入保留：{:?}", rep);
    let rec = load_record(&home);
    assert!(
        rec.grants
            .iter()
            .any(|g| g.sid == base_text && g.scope == GrantScope::Machine),
        "对账后机器级条目要还在台账里"
    );
    assert!(
        has_ace_for(base_sid, &target, RIGHTS_STAT),
        "对账不得撤机器级 ACE"
    );
    clean(&home).expect("收尾回收应当成功");
    let rec = load_record(&home);
    assert!(rec.grants.is_empty(), "收尾后机器级条目也要销账");
    assert!(rec.snapshots.is_empty(), "收尾后快照也要销账");
    free_sid(base_sid);
    discard(&base);
}

/// 【台账兼容】旧台账没有 scope 字段：读出来必须仍是按次口径（不能默认成机器级、被永久保留）。
#[test]
fn ledger_without_scope_field_loads_as_per_call() {
    let base = std::env::temp_dir().join(format!("solomni-scope-default-{}", std::process::id()));
    let home = base.join(".home");
    std::fs::create_dir_all(&home).expect("建台账目录");
    // 旧格式：GrantEntry 没有 scope 这一位。
    let legacy = r#"{
  "profiles": [],
  "grants": [
    {
      "sid": "S-1-15-2-1",
      "path": "C:/legacy/target",
      "rights": 128,
      "at": 1,
      "recursive": false,
      "owners": []
    }
  ],
  "snapshots": []
}"#;
    std::fs::write(record_path(&home), legacy).expect("写旧台账");
    let rec = load_record(&home);
    assert_eq!(rec.grants.len(), 1, "旧台账要能加载");
    assert_eq!(
        rec.grants[0].scope,
        GrantScope::Call,
        "旧条目缺 scope 时必须按按次处理"
    );
    assert_eq!(
        entry_state(&rec.grants[0]),
        OwnersState::Unjudgeable,
        "按次且缺归属仍是跳过，口径没有被悄悄改成机器级"
    );
    discard(&base);
}
