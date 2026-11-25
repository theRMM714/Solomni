//! 目的：共享区冲突与同步：删除对改动的解法、外部编辑上主线、归档点。
//! 管：可执行解法、接受上游后收敛、同步记录、归档点取舍。
//! 不管：提交与推送（`work/commit.rs`、`work/push.rs`）。
//! 联动：`docs/storage/README.md`。

use crate::tests::workspace::support::*;

/// 目的：删除型冲突（本地删了、主线改了）被拒时，回执给的补救路径要真能走通。
#[test]
pub(crate) fn work_push_delete_vs_modify_is_rejected_with_a_working_remedy() {
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v1\n").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap();
    push(&ws, "w", "a", "v1", 1, 1).unwrap();
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_b, "note.md", b"v2\n").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();
    push(&ws, "w", "b", "v2", 2, 2).unwrap();

    store.remove_under(&sb_a, "note.md").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 3))
        .unwrap();
    let err = push(&ws, "w", "a", "删掉", 3, 3).expect_err("删除不能覆盖上游修改");
    assert!(err.contains("本地删了、主线改了"), "{err}");
    assert!(
        err.contains("take=upstream") && err.contains("work_reset"),
        "要给出两条真能走通的补救路径：{err}"
    );
    assert!(
        !err.contains("再 work_pull 取回主线版本，在沙箱里合并"),
        "这条补救路径走不通，不能建议：{err}"
    );
}

/// 目的：补救路径①（接受主线版本再推）真能收敛：净改动为零，工作线保留提交。
#[test]
pub(crate) fn work_accept_upstream_after_a_delete_converges() {
    use crate::units::workspace::api::ResolveTake;
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v1\n").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap();
    push(&ws, "w", "a", "v1", 1, 1).unwrap();
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_b, "note.md", b"v2\n").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();
    push(&ws, "w", "b", "v2", 2, 2).unwrap();
    store.remove_under(&sb_a, "note.md").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 3))
        .unwrap();
    assert!(push(&ws, "w", "a", "删掉", 3, 3).is_err());

    ws.work_resolve("w", "a", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(
        store.read_under(&sb_a, "note.md").unwrap().as_deref(),
        Some(&b"v2\n"[..]),
        "接受主线版本：主副本那一版回到沙箱"
    );
    ws.work_commit(
        "w",
        "a",
        "w--a",
        &commit_req(&["note.md"], &[], "接受主线版本", 4),
    )
    .unwrap();

    let rep = push(&ws, "w", "a", "空推", 5, 4).unwrap();
    assert!(rep.commit.is_none(), "净改动为零：{:?}", rep);
    assert_eq!(rep.unpushed, 2, "工作线保留删除与恢复两条提交：{:?}", rep);
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.pending, 0, "内容与主线一致：{:?}", st);
    assert_eq!(st.conflicts, 0, "{:?}", st);
}

/// 目的：补救路径②（撤回删除提交再拉主线版本）真能收敛：工作线退回主线祖先，不再领先。
#[test]
pub(crate) fn work_reset_before_the_delete_then_pull_converges() {
    use crate::units::workspace::api::ResetRequest;
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v1\n").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap();
    push(&ws, "w", "a", "v1", 1, 1).unwrap();
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_b, "note.md", b"v2\n").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();
    push(&ws, "w", "b", "v2", 2, 2).unwrap();

    // 工作线头要从 work_status 取：work_head 给的是主副本那条线的头。
    let before_delete = ws
        .work_status("w", "a", &[])
        .unwrap()
        .work_head
        .expect("推送后工作线有头")
        .id;
    store.remove_under(&sb_a, "note.md").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 3))
        .unwrap();
    assert!(push(&ws, "w", "a", "删掉", 3, 3).is_err());

    ws.work_reset(
        "w",
        "a",
        &ResetRequest {
            paths: Vec::new(),
            to: Some(before_delete),
            keep_changes: false,
        },
    )
    .unwrap();
    let pulled = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert!(
        pulled.updated.contains(&"note.md".to_string()),
        "撤回删除后基线还是 v1，主副本的 v2 能拉进来：{:?}",
        pulled
    );
    assert_eq!(
        store.read_under(&sb_a, "note.md").unwrap().as_deref(),
        Some(&b"v2\n"[..])
    );

    let rep = push(&ws, "w", "a", "空推", 5, 4).unwrap();
    assert!(rep.commit.is_none(), "没有领先的改动：{:?}", rep);
    assert_eq!(rep.unpushed, 0, "工作线退回了主线的祖先：{:?}", rep);
}

/// 目的：`work_sync` 把用户在产品之外改的磁盘内容记成一条主线提交，不给 external 建工作线。
#[test]
pub(crate) fn work_sync_records_external_edits_on_the_main_line() {
    use crate::units::workspace::api::WorkState;
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let shared = shared_of(&ws, "w");
    let sandbox_a = sandbox_of(&ws, "w", "a");
    let store_dir = ws.roots("w", &[]).unwrap().store;

    // 用户在共享区目录里直接建了文件（产品之外的动作）。
    store.write_under(&shared, "note.md", b"v1\n").unwrap();
    let first = sync_at(&ws, "w", &["note.md"], 1, 5)
        .unwrap()
        .commit
        .expect("新增内容要落成一条提交");
    assert_eq!(ws.work_head("w").unwrap(), Some(first), "同步推进主线");
    assert!(
        store
            .read_ref(&store_dir, "agents/external")
            .unwrap()
            .is_none(),
        "external 不是 agent，不该给它建工作线"
    );
    let log = ws.work_log("w", 10, None).unwrap();
    let synced = log
        .commits
        .iter()
        .find(|c| c.id == first)
        .expect("同步提交进主线历史");
    assert_eq!(synced.author, "external");
    assert!(synced.on_head);
    assert!(log.work.is_empty(), "同步不产生工作线提交");

    // 外部又改了同一份文件：再同步一次，主线上多一条修改提交。
    store.write_under(&shared, "note.md", b"v2\n").unwrap();
    let second = sync_at(&ws, "w", &["note.md"], 2, 6)
        .unwrap()
        .commit
        .expect("外部修改要落成提交");
    assert_eq!(ws.work_head("w").unwrap(), Some(second));
    assert!(second > first);
    assert!(
        store
            .read_ref(&store_dir, "agents/external")
            .unwrap()
            .is_none(),
        "重复同步也不建工作线"
    );

    // 同步只改主线：沙箱里没有这个文件；没拉过的 agent 看到「主线有、还没拉取」。
    assert!(
        store.read_under(&sandbox_a, "note.md").unwrap().is_none(),
        "同步不改任何沙箱"
    );
    let st = ws.work_status("w", "a", &[]).unwrap();
    let entry = st
        .entries
        .iter()
        .find(|e| e.path == "note.md")
        .expect("同步进来的路径要在状态里");
    assert_eq!(entry.state, WorkState::NotPulled);
}

/// 目的：留档点不落在保留行之后的外来同步上——共享区回到那次推送，而不是用户后来的改动。
#[test]
pub(crate) fn work_archive_point_skips_a_sync_after_the_keep_line() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");
    store.write_under(&sb, "note.md", b"v119").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v119", 119);
    r.line = 119;
    ws.work_commit("w", "a", "s", &r).unwrap();
    let pushed = push(&ws, "w", "a", "v119", 119, 119)
        .unwrap()
        .commit
        .unwrap();

    // 用户在产品之外改了共享区，同步发生在第 200 行：它在这次留档的保留行之后。
    store.write_under(&shared, "note.md", b"external").unwrap();
    let synced = sync_at(&ws, "w", &["note.md"], 200, 200)
        .unwrap()
        .commit
        .unwrap();
    assert!(synced > pushed, "同步是主线上更晚的一条提交");

    let mut keep = std::collections::BTreeMap::new();
    keep.insert(String::new(), 119u64);
    keep.insert("a".to_string(), 119u64);
    let report = ws.work_archive("w", &keep, &["a".to_string()], 1).unwrap();
    assert_eq!(
        report.point,
        Some(pushed),
        "锚在保留行之后的同步不能算进留档点"
    );
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v119"[..]),
        "留档把共享区物化回那次推送的内容"
    );
}

/// 目的：同步锚在保留行之内时它是过去的一部分——留档点取这条同步。
#[test]
pub(crate) fn work_archive_point_takes_a_sync_before_the_keep_line() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    ws.work_commit("w", "a", "s", &r).unwrap();
    push(&ws, "w", "a", "v1", 2, 2).unwrap();

    store.write_under(&shared, "note.md", b"external").unwrap();
    let synced = sync_at(&ws, "w", &["note.md"], 3, 2)
        .unwrap()
        .commit
        .unwrap();

    let mut keep = std::collections::BTreeMap::new();
    keep.insert(String::new(), 2u64);
    keep.insert("a".to_string(), 2u64);
    let report = ws.work_archive("w", &keep, &["a".to_string()], 1).unwrap();
    assert_eq!(report.point, Some(synced), "同步在第 2 行，算在留档点之内");
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"external"[..])
    );
}

/// 目的：恢复把工作线放回留档时它自己的头——留档时被压制的工作线提交不会被丢掉。
#[test]
pub(crate) fn work_restore_archive_puts_the_work_line_back_to_its_archived_head() {
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let root = ws.roots("w", &[]).unwrap().store;
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    ws.work_commit("w", "a", "s", &r).unwrap();
    push(&ws, "w", "a", "v1", 2, 2).unwrap();

    // 第 6 行只提交、不推送：工作线比主线多一条。
    store.write_under(&sb, "note.md", b"v2").unwrap();
    let mut r2 = commit_req(&["note.md"], &[], "v2", 3);
    r2.line = 6;
    let unpushed = ws.work_commit("w", "a", "s", &r2).unwrap().id;

    let mut keep = std::collections::BTreeMap::new();
    keep.insert(String::new(), 2u64);
    keep.insert("a".to_string(), 2u64);
    ws.work_archive("w", &keep, &["a".to_string()], 1).unwrap();

    ws.work_restore_archive("w", 1).unwrap();
    assert_eq!(
        store
            .read_ref(&root, "agents/a")
            .unwrap()
            .and_then(|r| r.target),
        Some(unpushed),
        "工作线回到留档前它自己的头，而不是主线头"
    );
}
