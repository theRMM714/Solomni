//! 目的：共享区版本化工作流——状态、拉取、版本、提交。
//! 管：过期判定、提交历史、回滚选点、paths 入口、媒体资产、按根+增量落盘。
//! 不管：冲突解决（`resolve.rs`）。
//! 联动：`docs/storage/README.md`。

use super::super::prelude::*;
use super::support::*;

/// 目的：过期判定按内容——指纹相同不过期，改到读过的行才过期。
#[test]
pub(crate) fn work_stale_reads_judges_by_content_not_by_clock() {
    use crate::capabilities::workspace::api::{content_hash, ReadQuery};
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let v1 = b"a\nb\nc\n";
    ws.work_commit_user("w", "note.md", v1, 1, 0).unwrap();
    let q = |from: u64, to: u64, blob: String| ReadQuery {
        path: "note.md".to_string(),
        blob,
        from,
        to,
    };
    let old = content_hash(v1);

    // 内容与读的时候完全一样：不过期。
    assert_eq!(
        ws.work_stale_reads("w", &[q(1, 3, old.clone())]).unwrap(),
        vec![false]
    );

    // 只改第 3 行：读 1-2 不受影响，读 3-3 过期。
    ws.work_commit_user("w", "note.md", b"a\nb\nC\n", 2, 0)
        .unwrap();
    assert_eq!(
        ws.work_stale_reads(
            "w",
            &[
                q(1, 2, old.clone()),
                q(3, 3, old.clone()),
                q(9, 9, old.clone())
            ]
        )
        .unwrap(),
        vec![false, true, false],
        "区间不与改动相交 = 那次读仍然可信"
    );

    // 文件头插一行：行号全移了，但读过的那几行一个字没变 → 不算过期。
    ws.work_commit_user("w", "note.md", b"zero\na\nb\nC\n", 3, 0)
        .unwrap();
    assert_eq!(
        ws.work_stale_reads("w", &[q(1, 2, old.clone())]).unwrap(),
        vec![false]
    );

    // 文件被删：读过的东西不在了。
    ws.work_commit_user("w", "gone.md", b"x\n", 4, 0).unwrap();
    assert_eq!(
        ws.work_stale_reads(
            "w",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
            }]
        )
        .unwrap(),
        vec![false]
    );
    ws.work_pull("w", "a", &["gone.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    crate::capabilities::workspace::ports::WorkStore::remove_under(&*_store, &sb, "gone.md")
        .unwrap();
    ws.work_commit_auto("w", "a", "s", &commit_req(&[], &["gone.md"], "删", 5))
        .unwrap()
        .expect("应当发布一次删除");
    assert_eq!(
        ws.work_stale_reads(
            "w",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
            }]
        )
        .unwrap(),
        vec![true]
    );
}

/// 目的：三方比较的核心承诺：a、b 同基线各改同一文件，后者提交必须**报冲突**而不是覆盖；任一提交点都能还原出当时的共享区内容。
#[test]
pub(crate) fn work_commit_reports_conflicts_and_restore_returns_a_commit_point() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
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

    // a 改并提交 → 提交点 2。
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v2").unwrap();
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a 改", 2))
        .expect("a 的提交");
    assert_eq!(rep.id, 2);

    // b 基于 v1 改写为 v3：上游已变 → 冲突，整个提交被拒。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "note.md", b"v3").unwrap();
    let err = ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 3))
        .unwrap_err();
    assert!(
        err.contains("note.md") && err.contains("冲突"),
        "要逐条点名冲突：{}",
        err
    );
    // 冲突没有留下半个提交：head 仍在 2。
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
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
    ws.prepare("w", &[]).unwrap();
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
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");
    let root = ws.roots("w", &[]).unwrap().store;

    // 第 1 行：a 提交 v1。
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let mut r1 = commit_req(&["note.md"], &[], "v1", 1);
    r1.line = 1;
    let id1 = ws.work_commit("w", "a", "w--a", &r1).unwrap().id;

    // 第 3 行：a 再提交 v3。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let mut r3 = commit_req(&["note.md"], &[], "v3", 3);
    r3.line = 3;
    let id3 = ws.work_commit("w", "a", "w--a", &r3).unwrap().id;
    assert_eq!((id1, id3), (1, 2));

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
    ws.prepare("w", &["a".to_string(), "b".to_string()])
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
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");

    store.write_under(&sb, "a.txt", b"one").unwrap();
    let mut r1 = commit_req(&["a.txt"], &[], "一", 1);
    r1.line = 1;
    let id1 = ws.work_commit("w", "a", "w--a", &r1).unwrap().id;

    store.write_under(&sb, "a.txt", b"two").unwrap();
    let mut r2 = commit_req(&["a.txt"], &[], "二", 2);
    r2.line = 3;
    let id2 = ws.work_commit("w", "a", "w--a", &r2).unwrap().id;

    // 回到 id1，再从当前线继续提交（先拉齐沙箱，否则会被三方比较判冲突）。
    ws.work_restore("w", id1).unwrap();
    ws.work_pull("w", "a", &["a.txt".to_string()]).unwrap();
    store.write_under(&sb, "a.txt", b"three").unwrap();
    let mut r3 = commit_req(&["a.txt"], &[], "三", 3);
    r3.line = 5;
    let id3 = ws.work_commit("w", "a", "w--a", &r3).unwrap().id;

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
    ws.prepare("w", &["a".to_string()]).unwrap();
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
    ws.prepare("w", &["a".to_string(), "b".to_string()])
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
    ws.prepare("w", &["a".to_string()]).unwrap();
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

/// 目的：work_status 把"本地有修改 / 上游有更新 / 冲突"分开说，而不是笼统一句。
#[test]
pub(crate) fn work_status_separates_local_and_upstream_changes() {
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries.len(), 1);
    assert_eq!(st.entries[0].state, StatusState::Clean);

    store.write_under(&sb, "note.md", b"v2").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::LocalModified);

    ws.work_commit_user("w", "note.md", b"v3", 2, 0).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Conflict);
    assert!(st.head.is_some());
    assert!(st.tree.contains(&"note.md".to_string()));
}

/// 目的：删除必须显式：path 从沙箱消失不等于删共享文件；deletes 会真的从主副本删掉。
#[test]
pub(crate) fn work_delete_is_explicit_in_the_commit_request() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(
        rep.changes[0].kind,
        crate::capabilities::workspace::api::ChangeKind::Delete
    );
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(!st.tree.contains(&"note.md".to_string()));
}

/// 目的：work_* 的 paths 入口把“写错”和“上游没有”分开报；空提交说清真实原因；沙箱里删掉已拉取的文件时状态是“本地已删除”，不是“本地有修改”。
#[test]
pub(crate) fn work_paths_and_empty_commit_report_the_real_reason() {
    use crate::capabilities::workspace::domain::workstore::StatusState;
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    // 非法路径整条拒绝，不再降级成“主副本没有”。
    let bad = ws
        .work_pull("w", "a", &["../x.txt".to_string()])
        .unwrap_err();
    assert!(bad.contains("不能包含 . 或 .."), "{}", bad);
    let bad = ws
        .work_commit("w", "a", "w--a", &commit_req(&["../x.txt"], &[], "x", 2))
        .unwrap_err();
    assert!(bad.contains("不能包含 . 或 .."), "{}", bad);
    let bad = ws
        .work_status("w", "a", &["../x.txt".to_string()])
        .unwrap_err();
    assert!(bad.contains("不能包含 . 或 .."), "{}", bad);

    // 合法但不存在：work_status 如实回报 missing，不静默丢。
    let st = ws.work_status("w", "a", &["nope.txt".to_string()]).unwrap();
    assert_eq!(st.missing, vec!["nope.txt"]);

    // 上游删除、本地未改：空提交说“上游已删除”，不再说“与主副本一致”。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&[], &["note.md"], "删", 2))
        .unwrap();
    let empty = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "重加", 3))
        .unwrap_err();
    assert!(empty.contains("上游已删除"), "{}", empty);
    assert!(!empty.contains("与主副本一致"), "{}", empty);

    // 本地删除（上游没动）→ 本地已删除。
    ws.prepare("w2", &["a".to_string()]).unwrap();
    ws.work_commit_user("w2", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w2", "a", &["note.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w2", "a");
    store.remove_under(&sb, "note.md").unwrap();
    let st = ws.work_status("w2", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::LocalDeleted);
}

/// 目的：work_versions 给文本预览、二进制只给指纹与大小；不把二进制冒充文本。
#[test]
pub(crate) fn work_versions_reports_text_and_binary_without_guessing() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let binary: &[u8] = &[0, 159, 146, 150, 0, 255];
    ws.work_commit_user("w", "image.bin", binary, 2, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string(), "image.bin".to_string()])
        .unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "image.bin", &[0, 1, 2]).unwrap();

    let v = ws.work_versions("w", "a", "image.bin").unwrap();
    assert_eq!(v.path, "image.bin");
    assert!(v.media_type.is_none(), "未知扩展名没有类型提示");
    assert!(v.base.is_some() && v.head.is_some() && v.local.is_some());
    let head = v.head.unwrap();
    assert!(head.text.is_none(), "二进制不能给文本预览");
    assert_eq!(head.size, binary.len() as u64);

    let note = ws.work_versions("w", "a", "note.md").unwrap();
    assert_eq!(note.media_type.as_deref(), Some("text/markdown"));
    assert_eq!(
        note.head_commit,
        Some(2),
        "work_versions 要给出 take=local 的 head"
    );
    assert_eq!(note.head.unwrap().text.as_deref(), Some("v1"));
    assert!(ws.work_versions("w", "a", "nope.bin").is_err());
}

/// 目的：work_versions 按扩展名给媒体类型；既有超限对象只给指纹/大小/类型，不再读内容。
#[test]
pub(crate) fn work_versions_reports_media_type_and_marks_oversize_metadata_only() {
    let dirs = Arc::new(InMemoryWorkspace::new());
    let store = Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store_with_max(dirs.clone(), store.clone(), 1024);
    ws.prepare("w", &["a".to_string()]).unwrap();
    let png = [0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    ws.work_commit_user("w", "cover.png", &png, 1, 0).unwrap();
    ws.work_pull("w", "a", &["cover.png".to_string()]).unwrap();
    let v = ws.work_versions("w", "a", "cover.png").unwrap();
    assert_eq!(v.media_type.as_deref(), Some("image/png"));
    let head = v.head.unwrap();
    assert!(head.text.is_none(), "已知媒体不给文本预览");
    assert_eq!(head.size, png.len() as u64);

    // 用更小的上限重新装配同一个版本库：既有对象只给元数据，不读内容。
    let strict = test_workspace_store_with_max(dirs, store.clone(), 4);
    let v = strict.work_versions("w", "a", "cover.png").unwrap();
    assert_eq!(v.media_type.as_deref(), Some("image/png"));
    let head = v.head.unwrap();
    assert!(head.too_large, "超过新上限要标记");
    assert!(head.text.is_none());
    assert_eq!(head.size, png.len() as u64);

    // 超限提交与上传都被拒绝；head 不前进。
    let before = strict.work_head("w").unwrap();
    let sb = sandbox_of(&strict, "w", "a");
    store.write_under(&sb, "big.bin", &[0u8; 5]).unwrap();
    let err = strict
        .work_commit("w", "a", "w--a", &commit_req(&["big.bin"], &[], "big", 2))
        .unwrap_err();
    assert!(err.contains("上限") && err.contains("big.bin"), "{}", err);
    let err = strict
        .work_commit_user("w", "up.bin", &[0u8; 5], 3, 0)
        .unwrap_err();
    assert!(err.contains("上限"), "{}", err);
    assert_eq!(strict.work_head("w").unwrap(), before, "拒绝不能推进 head");
}

/// 目的：媒体资产的冲突也只能显式选边；三方视图按媒体类型给元数据，不做内容合并。
#[test]
pub(crate) fn work_media_conflict_requires_explicit_side() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let base: &[u8] = &[0, 0, 0, 24, b'f', b't', b'y', b'p'];
    ws.work_commit_user("w", "clip.mp4", base, 1, 0).unwrap();
    let both = ["clip.mp4".to_string()];
    ws.work_pull("w", "a", &both).unwrap();
    ws.work_pull("w", "b", &both).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_a, "clip.mp4", &[1, 2, 3]).unwrap();
    store.write_under(&sb_b, "clip.mp4", &[4, 5, 6]).unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["clip.mp4"], &[], "a", 2))
        .unwrap();
    let err = ws
        .work_commit("w", "b", "w--b", &commit_req(&["clip.mp4"], &[], "b", 3))
        .unwrap_err();
    assert!(err.contains("clip.mp4") && err.contains("冲突"), "{}", err);

    let v = ws.work_versions("w", "b", "clip.mp4").unwrap();
    assert_eq!(v.media_type.as_deref(), Some("video/mp4"));
    assert!(v.head.unwrap().text.is_none(), "媒体不给文本");
    assert!(v.local.unwrap().text.is_none(), "媒体不给文本");

    let rep = ws
        .work_resolve("w", "b", &resolve_req(&["clip.mp4"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::WroteUpstream);
    assert_eq!(
        store.read_under(&sb_b, "clip.mp4").unwrap().as_deref(),
        Some(&[1, 2, 3][..])
    );
}

/// 目的：资产路径（pull / status / versions / commit / resolve / restore）不走整文件读取。
#[test]
pub(crate) fn work_asset_paths_do_not_use_whole_file_reads() {
    use crate::capabilities::workspace::api::ResolveTake;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    // 之后任何整文件读取都会失败：生产资产路径必须走 stat / scan / ingest / copy。
    store.fail_times("read_under", 100);
    store.fail_times("read_object", 100);

    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();
    let _ = ws.work_status("w", "a", &[]).unwrap();
    let _ = ws.work_versions("w", "a", "note.md").unwrap();
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    let _ = ws
        .work_resolve("w", "a", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    ws.work_restore("w", rep.id).unwrap();
}

/// 目的：提交按"根 + 增量"落盘，读树仍给出完整结果；work_status 的树由回溯重建。
/// 约束：只有根提交带整棵树，增量只记 changes。
#[test]
pub(crate) fn work_commits_store_roots_and_deltas_but_read_full_tree() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "a.md", b"1", 1, 0).unwrap();
    ws.work_commit_user("w", "b.md", b"2", 2, 0).unwrap();
    ws.work_commit_user("w", "c.md", b"3", 3, 0).unwrap();
    let sd = ws.roots("w", &[]).unwrap().store;

    let c1 = store.read_commit(&sd, 1).unwrap().unwrap();
    let c2 = store.read_commit(&sd, 2).unwrap().unwrap();
    let c3 = store.read_commit(&sd, 3).unwrap().unwrap();
    assert!(c1.tree.is_some(), "根提交带整棵树");
    assert!(c2.tree.is_none(), "增量提交不落整棵树");
    assert!(c3.tree.is_none(), "增量提交不落整棵树");
    assert_eq!((c1.depth, c2.depth, c3.depth), (0, 1, 2));
    assert_eq!((c2.base, c3.base), (1, 2));

    // 树由根 + 沿途增量重建：三份文件都在。
    let st = ws.work_status("w", "a", &[]).unwrap();
    let mut tree = st.tree.clone();
    tree.sort();
    assert_eq!(
        tree,
        vec!["a.md".to_string(), "b.md".to_string(), "c.md".to_string()]
    );
}
