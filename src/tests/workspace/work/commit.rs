//! 目的：共享区提交与历史：提交、日志与预览、回滚到提交点、拉取基线、删除。
//! 管：提交请求回执、离线索可见性、重置选点、基线持久化与删除显式化。
//! 不管：状态与版本视图（`work/status.rs`）、推送与差异（`work/push.rs`）。
//! 联动：`docs/storage/README.md`。

use crate::tests::workspace::support::*;

/// 目的：三方比较的核心承诺：a、b 同基线各改同一文件，后者提交必须**报冲突**而不是覆盖；任一提交点都能还原出当时的共享区内容。
#[test]
pub(crate) fn work_commit_reports_conflicts_and_restore_returns_a_commit_point() {
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();

    // a、b 都对准同一版（v1）。
    assert_eq!(
        ws.work_pull("w", "a", &["note.md".to_string()])
            .unwrap()
            .updated,
        vec!["note.md"]
    );
    assert_eq!(
        ws.work_pull("w", "b", &["note.md".to_string()])
            .unwrap()
            .updated,
        vec!["note.md"]
    );

    // a 改、提交、推送 → 共享区到 v2。
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v2").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a 改", 2))
        .expect("a 的提交");
    let published = push(&ws, "w", "a", "a 改", 2, 2)
        .expect("a 的推送")
        .commit
        .expect("推送要落主线提交");

    // b 基于 v1 改写为 v3：本地提交照收（还没碰共享区），推送时三方比较才发现冲突。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "note.md", b"v3").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 3))
        .expect("本地提交不看别人");
    let err = push(&ws, "w", "b", "b 改", 3, 3).unwrap_err();
    assert!(
        err.contains("note.md") && err.contains("冲突"),
        "要逐条点名冲突：{}",
        err
    );
    // 冲突没有动共享区：head 仍是 a 推送的那个点。
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(published)
    );

    // 还原到提交点 1：共享区内容回到 v1。
    ws.work_restore("w", 1).unwrap();
    let shared = shared_of(&ws, "w");
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
}

/// 目的：提交历史与提交详情：列表按新→旧、on_head 标当前线；详情给改动大小；文件预览文本/二进制分开。
/// 约束：非破坏性回滚后旧提交仍在列表里（on_head=false），从旧点继续提交会形成新分支。
#[test]
pub(crate) fn work_log_and_commit_preview_keep_off_line_commits_visible() {
    let (ws, _dirs, _store) = work_ws();
    ws.ensure_layout("w", &[]).unwrap();
    let id1 = ws.work_commit_user("w", "a.txt", b"one", 10, 0).unwrap();
    let id2 = ws
        .work_commit_user("w", "b.bin", &[0, 1, 2], 11, 0)
        .unwrap();

    let log = ws.work_log("w", 10, None).unwrap();
    assert_eq!(log.head.as_ref().map(|h| h.id), Some(id2));
    assert_eq!(
        log.commits.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![id2, id1]
    );
    assert!(
        log.commits.iter().all(|c| c.on_head),
        "两个提交都在当前线上"
    );
    assert_eq!(log.total, 2);
    assert!(!log.truncated);

    let d = ws.work_commit_view("w", id1).unwrap();
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].path, "a.txt");
    assert_eq!(d.changes[0].size, 3);
    assert_eq!(d.changes[0].media_type.as_deref(), Some("text/plain"));
    let f = ws.work_commit_file("w", id1, "a.txt").unwrap();
    assert_eq!(f.text.as_deref(), Some("one"));
    assert!(!f.truncated && !f.too_large);
    let b = ws.work_commit_file("w", id2, "b.bin").unwrap();
    assert_eq!(b.text, None, "含 NUL 的二进制不给文本预览");
    assert_eq!(b.size, 3);
    assert!(
        ws.work_commit_file("w", id1, "nope.txt").is_err(),
        "路径必须真在这条提交的树里"
    );

    // 非破坏性回滚：head 回到 id1，id2 仍在历史里，但标成"已离开当前线"。
    ws.work_restore("w", id1).unwrap();
    let back = ws.work_log("w", 10, None).unwrap();
    assert_eq!(back.head.as_ref().map(|h| h.id), Some(id1));
    assert_eq!(
        back.commits
            .iter()
            .map(|c| (c.id, c.on_head))
            .collect::<Vec<_>>(),
        vec![(id2, false), (id1, true)]
    );

    // 从旧点继续提交：新提交的 parent = id1，旧 id2 仍保留（可再次回滚）。
    let id3 = ws.work_commit_user("w", "a.txt", b"three", 12, 0).unwrap();
    let branched = ws.work_log("w", 10, None).unwrap();
    assert_eq!(branched.head.as_ref().map(|h| h.id), Some(id3));
    assert_eq!(
        branched
            .commits
            .iter()
            .map(|c| (c.id, c.on_head))
            .collect::<Vec<_>>(),
        vec![(id3, true), (id2, false), (id1, true)]
    );
    assert_eq!(branched.commits[0].parent, Some(id1));

    // before 分页：只取 id 更小的提交。
    let page = ws.work_log("w", 1, Some(id3)).unwrap();
    assert_eq!(
        page.commits.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![id2]
    );
    assert!(page.truncated, "后面还有 id1");
}

/// 目的：按行锚精确物化共享区：agent 第 1 行提交 v1、第 3 行提交 v3；回档到第 1 行 → 共享区回到 v1；再丢弃非祖先提交 → 第 3 行的提交记录真的没了。
#[test]
pub(crate) fn work_rewind_to_materializes_by_line_anchor_and_discards_later_commits() {
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");
    let root = ws.roots("w", &[]).unwrap().store;

    // 第 1 行：a 提交 v1 并推送，主线落一个锚在第 1 行的提交。
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let mut r1 = commit_req(&["note.md"], &[], "v1", 1);
    r1.line = 1;
    ws.work_commit("w", "a", "w--a", &r1).unwrap();
    let id1 = push(&ws, "w", "a", "v1", 1, 1)
        .unwrap()
        .commit
        .expect("推送要落主线提交");

    // 第 3 行：a 再提交 v3 并推送。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let mut r3 = commit_req(&["note.md"], &[], "v3", 3);
    r3.line = 3;
    ws.work_commit("w", "a", "w--a", &r3).unwrap();
    let id3 = push(&ws, "w", "a", "v3", 3, 3)
        .unwrap()
        .commit
        .expect("推送要落主线提交");
    assert!(id3 > id1);

    // 回档到第 1 行（保留行 = 1）：只保留锚 ≤ 1 的提交 → v1。
    let mut keep = std::collections::BTreeMap::new();
    keep.insert("a".to_string(), 1u64);
    assert_eq!(ws.work_head("w").unwrap(), Some(id3));
    assert_eq!(ws.work_rewind_to("w", &keep).unwrap(), Some(id1));
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));

    // 丢弃第 1 行之后的提交：第 3 行的提交记录真的没了。
    ws.work_discard_after("w", Some(id1)).unwrap();
    assert_eq!(store.list_commits(&root).unwrap(), vec![id1]);

    // 恢复到空共享区。
    ws.work_restore_point("w", None).unwrap();
    assert!(store.read_under(&shared, "note.md").unwrap().is_none());
    assert_eq!(ws.work_head("w").unwrap(), None);
}

/// 目的：用户回滚共享区后，新开的 agent 拉取到的是目标提交点的内容。
#[test]
pub(crate) fn work_restore_is_visible_to_a_new_agent_on_pull() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let id1 = ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();
    assert_ne!(id1, id2);

    // 回滚到 v1；b 从没拉取过，之后首次拉取必须看到 v1。
    ws.work_restore("w", id1).unwrap();
    let pulled = ws
        .work_pull("w", "b", &["note.md".to_string()])
        .expect("新 agent 首次拉取");
    assert_eq!(
        pulled.updated,
        vec!["note.md"],
        "回滚后的 head 内容写进新沙箱"
    );
    let sb = sandbox_of(&ws, "w", "b");
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
}

/// 目的：回滚离开过的提交不在当前 head 祖先链上：转录回档只沿当前线选点，不能把旧分支选回来。
#[test]
pub(crate) fn work_rewind_to_ignores_commits_off_the_current_head() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");

    store.write_under(&sb, "a.txt", b"one").unwrap();
    let mut r1 = commit_req(&["a.txt"], &[], "一", 1);
    r1.line = 1;
    ws.work_commit("w", "a", "w--a", &r1).unwrap();
    let id1 = push(&ws, "w", "a", "一", 1, 1).unwrap().commit.unwrap();

    store.write_under(&sb, "a.txt", b"two").unwrap();
    let mut r2 = commit_req(&["a.txt"], &[], "二", 2);
    r2.line = 3;
    ws.work_commit("w", "a", "w--a", &r2).unwrap();
    let id2 = push(&ws, "w", "a", "二", 2, 3).unwrap().commit.unwrap();

    // 回到 id1，再从当前线继续提交、推送：id2 就离开了当前线。
    ws.work_restore("w", id1).unwrap();
    ws.work_pull("w", "a", &["a.txt".to_string()]).unwrap();
    store.write_under(&sb, "a.txt", b"three").unwrap();
    let mut r3 = commit_req(&["a.txt"], &[], "三", 3);
    r3.line = 5;
    ws.work_commit("w", "a", "w--a", &r3).unwrap();
    let id3 = push(&ws, "w", "a", "三", 3, 5).unwrap().commit.unwrap();

    // 保留行 4：id2 的锚 3 会通过，但它已离开当前线；当前线上 id1(1) 通过、id3(5) 不过。
    let mut keep = std::collections::BTreeMap::new();
    keep.insert("a".to_string(), 4u64);
    assert_eq!(ws.work_head("w").unwrap(), Some(id3));
    assert_eq!(ws.work_rewind_to("w", &keep).unwrap(), Some(id1));
    assert_eq!(
        store.read_under(&shared, "a.txt").unwrap().as_deref(),
        Some(&b"one"[..])
    );
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));
    // id2 / id3 仍是历史（非破坏性）；这里只是不再被选为当前点。
    let log = ws.work_log("w", 10, None).unwrap();
    assert_eq!(
        log.commits
            .iter()
            .map(|c| (c.id, c.on_head))
            .collect::<Vec<_>>(),
        vec![(id3, false), (id2, false), (id1, true)]
    );
}

/// 目的：用户投喂 = 权威提交；agent 拉取后按基线提交，新路径是新增。
#[test]
pub(crate) fn work_user_commit_is_authoritative_and_pull_tracks_the_baseline() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    // 权威覆盖：同名再投喂直接成为新的 head，不报冲突。
    let id = ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();
    assert_eq!(id, 2);

    let r = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(r.updated, vec!["note.md"]);
    let sb = sandbox_of(&ws, "w", "a");
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v2"[..])
    );

    // 沙箱里改写成本地基线之上的新内容 → modify。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let rep = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["note.md"], &[], "改成 v3", 3),
        )
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(rep.changes[0].path, "note.md");
}

/// 目的：拉取绝不静默覆盖本地已有内容；提交空集与空说明都如实拒绝。
#[test]
pub(crate) fn work_pull_never_clobbers_and_empty_commits_are_refused() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    const DRAFT: &[u8] = "本地草稿".as_bytes();
    const B_VERSION: &[u8] = "b 版".as_bytes();
    const USER_EDIT: &[u8] = "用户改".as_bytes();

    // 无基线的同沙箱路径：按"已占用"处理，不覆盖。
    store.write_under(&sb_a, "note.md", DRAFT).unwrap();
    let r = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(r.occupied, vec!["note.md"]);
    assert_eq!(
        store.read_under(&sb_a, "note.md").unwrap().as_deref(),
        Some(DRAFT)
    );

    // 有基线、本地改了、上游也改了 → 冲突，不覆盖本地。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_b, "note.md", B_VERSION).unwrap(); // 本地改
    ws.work_commit_user("w", "note.md", USER_EDIT, 2, 0)
        .unwrap(); // 上游也改
    let r = ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].path, "note.md");
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(B_VERSION)
    );

    // 空提交与空说明都被如实拒绝。
    let empty = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &[], "说明", 3))
        .unwrap_err();
    assert!(empty.contains("没有可提交的改动"), "{}", empty);
    let no_msg = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "   ", 3))
        .unwrap_err();
    assert!(no_msg.contains("提交说明"), "{}", no_msg);
    let bad_pull = ws.work_pull("w", "a", &[]).unwrap_err();
    assert!(bad_pull.contains("路径"), "{}", bad_pull);
}

/// 目的：pull 中途失败保留已完成部分的基线；上游再动后重拉不产生伪冲突。
#[test]
pub(crate) fn work_pull_persists_partial_baseline_on_later_failure() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "a.txt", b"a1", 1, 0).unwrap();
    ws.work_commit_user("w", "b.txt", b"b1", 2, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let both = ["a.txt".to_string(), "b.txt".to_string()];
    ws.work_pull("w", "a", &both).unwrap();

    ws.work_commit_user("w", "a.txt", b"a2", 3, 0).unwrap();
    ws.work_commit_user("w", "b.txt", b"b2", 4, 0).unwrap();
    store.fail_on("copy_object_to_under", "b.txt", 1);
    let err = ws.work_pull("w", "a", &both).unwrap_err();
    assert!(
        err.contains("b.txt") && err.contains("已写入/登记 1 个"),
        "{}",
        err
    );
    assert_eq!(
        store.read_under(&sb, "a.txt").unwrap().as_deref(),
        Some(&b"a2"[..]),
        "a.txt 已经写进沙箱，基线也必须已经推进"
    );

    ws.work_commit_user("w", "a.txt", b"a3", 5, 0).unwrap();
    let rep = ws.work_pull("w", "a", &both).unwrap();
    assert!(
        rep.conflicts.is_empty(),
        "部分基线没保留时这里会报伪冲突：{:?}",
        rep.conflicts
    );
    assert!(rep.updated.contains(&"a.txt".to_string()));
    assert!(rep.updated.contains(&"b.txt".to_string()));
}

/// 目的：删除必须显式：path 从沙箱消失不等于删共享文件；deletes 会真的从主副本删掉。
#[test]
pub(crate) fn work_delete_is_explicit_in_the_commit_request() {
    let (ws, _dirs, _store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    // 本地提交只记「我要删」；共享区要到推送后才真的没有它。
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(
        rep.changes[0].kind,
        crate::units::business::versions::api::ChangeKind::Delete
    );
    assert!(ws.work_has("w", "note.md"), "没推送之前共享区里还在");
    push(&ws, "w", "a", "删掉", 2, 2).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(!st.tree.contains(&"note.md".to_string()));
}
