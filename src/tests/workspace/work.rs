//! 目的：共享区版本化工作流——状态、拉取、版本、提交。
//! 管：过期判定、提交历史、回滚选点、paths 入口、媒体资产、按根+增量落盘。
//! 不管：冲突解决（`resolve.rs`）。
//! 联动：`docs/storage/README.md`。

use super::super::prelude::*;
use super::support::*;

/// 目的：过期判定按内容——指纹相同不过期，改到读过的行才过期。
#[test]
pub(crate) fn work_stale_reads_judges_by_content_not_by_clock() {
    use crate::capabilities::workspace::api::{ReadQuery, content_hash};
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let v1 = b"a\nb\nc\n";
    ws.work_commit_user("w", "note.md", v1, 1, 0).unwrap();
    let q = |from: u64, to: u64, blob: String| ReadQuery {
        path: "note.md".to_string(),
        blob,
        from,
        to,
        sandbox: false,
    };
    let old = content_hash(v1);

    // 内容与读的时候完全一样：不过期。
    assert_eq!(
        ws.work_stale_reads("w", "a", &[q(1, 3, old.clone())])
            .unwrap(),
        vec![false]
    );

    // 只改第 3 行：读 1-2 不受影响，读 3-3 过期。
    ws.work_commit_user("w", "note.md", b"a\nb\nC\n", 2, 0)
        .unwrap();
    assert_eq!(
        ws.work_stale_reads(
            "w",
            "a",
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
        ws.work_stale_reads("w", "a", &[q(1, 2, old.clone())])
            .unwrap(),
        vec![false]
    );

    // 文件被删：读过的东西不在了。
    ws.work_commit_user("w", "gone.md", b"x\n", 4, 0).unwrap();
    assert_eq!(
        ws.work_stale_reads(
            "w",
            "a",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
                sandbox: false,
            }]
        )
        .unwrap(),
        vec![false]
    );
    ws.work_pull("w", "a", &["gone.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    _store.write_under(&sb, "draft.md", b"d").unwrap();
    // 存档只记沙箱快照：共享区里这个文件还在，所以读过它的内容还不算过期。
    ws.work_commit_auto("w", "a", "s", &commit_req(&["draft.md"], &[], "草稿", 5))
        .unwrap()
        .expect("应当记一次存档");
    assert_eq!(
        ws.work_stale_reads(
            "w",
            "a",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
                sandbox: false,
            }]
        )
        .unwrap(),
        vec![false]
    );
    // 发布之后共享区才真的没有这个文件——读过它内容的上下文立刻过期。
    crate::capabilities::workspace::ports::WorkStore::remove_under(&*_store, &sb, "gone.md")
        .unwrap();
    ws.work_commit("w", "a", "s", &commit_req(&[], &["gone.md"], "删", 6))
        .unwrap();
    // 工作线提交只是本地提交：共享区要到推送后才真的没有这个文件。
    push(&ws, "w", "a", "删", 6, 6).unwrap();
    assert_eq!(
        ws.work_stale_reads(
            "w",
            "a",
            &[ReadQuery {
                path: "gone.md".to_string(),
                blob: content_hash(b"x\n"),
                from: 1,
                to: 1,
                sandbox: false,
            }]
        )
        .unwrap(),
        vec![true]
    );
}

/// 目的：沙箱里的读按沙箱当前内容判——主副本改了不影响它，沙箱自己改了才过期。
#[test]
pub(crate) fn work_stale_reads_follows_the_sandbox_copy() {
    use crate::capabilities::workspace::api::{ReadQuery, content_hash};
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let v1 = b"one\ntwo\nthree\n";
    store.write_under(&sb, "draft.md", v1).unwrap();
    // 先记一次存档：正文进对象库，行区间才有得比（读工具读到的就是这类已入库内容）。
    ws.work_commit_auto("w", "a", "s", &commit_req(&["draft.md"], &[], "草稿", 1))
        .unwrap()
        .expect("应当记一次存档");
    let q = |from: u64, to: u64| ReadQuery {
        path: "draft.md".to_string(),
        blob: content_hash(v1),
        from,
        to,
        sandbox: true,
    };
    assert_eq!(
        ws.work_stale_reads("w", "a", &[q(1, 3)]).unwrap(),
        vec![false]
    );
    // 主副本里另有一份同名文件、内容不同：沙箱的读不该被它影响。
    ws.work_commit_user("w", "draft.md", b"main\n", 1, 0)
        .unwrap();
    assert_eq!(
        ws.work_stale_reads("w", "a", &[q(1, 3)]).unwrap(),
        vec![false],
        "主副本变了不代表沙箱里的读过期"
    );
    // 沙箱里只改第 2 行：读 1-1 不受影响，读 2-2 过期。
    store
        .write_under(&sb, "draft.md", b"one\nTWO\nthree\n")
        .unwrap();
    assert_eq!(
        ws.work_stale_reads("w", "a", &[q(1, 1), q(2, 2)]).unwrap(),
        vec![false, true]
    );
    // 沙箱里删掉：读过的东西不在了。
    store.remove_under(&sb, "draft.md").unwrap();
    assert_eq!(
        ws.work_stale_reads("w", "a", &[q(1, 3)]).unwrap(),
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

/// 目的：删除必须显式：path 从沙箱消失不等于删共享文件；deletes 会真的从主副本删掉。
#[test]
pub(crate) fn work_delete_is_explicit_in_the_commit_request() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    // 本地提交只记「我要删」；共享区要到推送后才真的没有它。
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(
        rep.changes[0].kind,
        crate::capabilities::workspace::api::ChangeKind::Delete
    );
    assert!(ws.work_has("w", "note.md"), "没推送之前共享区里还在");
    push(&ws, "w", "a", "删掉", 2, 2).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(!st.tree.contains(&"note.md".to_string()));
}

/// 目的：work_status 按「工作线 / 主线 / 沙箱」报五态：未提交新增 → 未推送 → 已提交 → 提交后又改 → 未提交删除；主线有、没拉取单独标。
#[test]
pub(crate) fn work_status_reports_the_work_line_five_states() {
    use crate::capabilities::workspace::api::{StatusReport, WorkState};
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let find =
        |st: &StatusReport, path: &str| st.entries.iter().find(|e| e.path == path).map(|e| e.state);
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "draft.md", b"v1").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Untracked));
    assert!(st.work_head.is_none() && st.unpushed == 0);

    let c1 = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["draft.md"], &[], "第一版", 1),
        )
        .unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Unpushed));
    assert_eq!(st.unpushed, 1);
    assert_eq!(st.work_head.as_ref().map(|c| c.id), Some(c1.id));

    let published = push(&ws, "w", "a", "第一版", 1, 1)
        .unwrap()
        .commit
        .expect("有内容可推送");
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Committed));
    assert_eq!(st.unpushed, 0);
    assert_eq!(st.head.as_ref().map(|c| c.id), Some(published));
    assert_eq!(st.work_head.as_ref().map(|c| c.id), Some(published));

    store.write_under(&sb, "draft.md", b"v2").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Modified));
    store.remove_under(&sb, "draft.md").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "draft.md"), Some(WorkState::Deleted));

    // b 推送另一条路径：a 还没拉取，状态应说「主线有、还没拉取」而不是「未提交新增」。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "another.md", b"x").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["another.md"], &[], "b", 2))
        .unwrap();
    push(&ws, "w", "b", "b", 2, 2).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(find(&st, "another.md"), Some(WorkState::NotPulled));

    // work_log 另列各 agent 工作线的提交，并标是否已推送。
    let log = ws.work_log("w", 10, None).unwrap();
    let mine = log
        .work
        .iter()
        .find(|e| e.id == c1.id)
        .expect("工作线提交在列");
    assert_eq!(mine.author, "a");
    assert!(mine.pushed, "已推送过");
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
    push(&ws, "w", "a", "a", 2, 2).unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["clip.mp4"], &[], "b", 3))
        .expect("本地提交不看别人");
    let err = push(&ws, "w", "b", "b", 3, 3).unwrap_err();
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

/// 目的：验成员侧工作工具执行者按角色面发放的名单现取——没发任何 `work_*` 就一个都不装。
#[test]
pub(crate) fn work_handlers_are_taken_only_when_the_role_face_allows_them() {
    use crate::capabilities::workspace::api::work_handlers;

    let (ws, _dirs, _store) = work_ws();
    let sb = test_sandbox("a", &[]);
    assert!(
        work_handlers(&ws, &sb, &[]).is_empty(),
        "一个 work_* 都没发就不装执行者"
    );
    assert!(
        work_handlers(&ws, &sb, &["read".to_string()]).is_empty(),
        "只发别的工具也不装"
    );
    let allowed = vec!["read".to_string(), "work_pull".to_string()];
    let handlers = work_handlers(&ws, &sb, &allowed);
    assert_eq!(handlers.len(), 1, "发了一个 work_* 就装一个执行者");
    assert!(handlers[0].owns("work_pull"));
    assert!(!handlers[0].owns("read"), "执行者不认领非 work_* 的名字");
}
/// 目的：自动提交是**存档**——它把沙箱改动记进该 agent 的存档线，共享区与主线一个字都不动。
/// 约束：存档线是 `refs/agents/<agent>/saves`；要进共享区仍得显式 work_commit。
#[test]
pub(crate) fn work_commit_auto_saves_the_sandbox_without_touching_the_shared_area() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"b", 1, 0).unwrap();
    let shared = shared_of(&ws, "w");
    let sd = ws.roots("w", &[]).unwrap().store;
    let head_before = store.head(&sd).unwrap();

    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "draft.md", b"draft").unwrap();
    let saved = ws
        .work_commit_auto("w", "a", "w--a", &commit_req(&["draft.md"], &[], "草稿", 2))
        .unwrap()
        .expect("应当记一次存档");

    assert_eq!(store.head(&sd).unwrap(), head_before, "存档不推进主线");
    assert!(
        store.stat_under(&shared, "draft.md").unwrap().is_none(),
        "存档不写共享区"
    );
    let saves = store.read_ref(&sd, "agents/a/saves").unwrap().unwrap();
    assert_eq!(saves.target, Some(saved.id), "存档线头指向这条提交");
    let c = store.read_commit(&sd, saved.id).unwrap().unwrap();
    assert_eq!((c.author.as_str(), c.session.as_str()), ("a", "w--a"));
    assert!(
        c.tree.as_ref().is_some_and(|t| t.contains_key("draft.md")),
        "存档提交带整棵树，回档才铺得出沙箱快照"
    );

    // 显式提交只进工作线；推送之后主线才前进，共享区才出现这个文件。
    ws.work_commit("w", "a", "w--a", &commit_req(&["draft.md"], &[], "发布", 3))
        .unwrap();
    assert!(
        store.stat_under(&shared, "draft.md").unwrap().is_none(),
        "工作线提交也不写共享区"
    );
    push(&ws, "w", "a", "发布", 3, 3).unwrap();
    assert!(
        store.stat_under(&shared, "draft.md").unwrap().is_some(),
        "发布才写共享区"
    );
    assert!(store.head(&sd).unwrap().unwrap() > head_before.unwrap());
}
/// 目的：推送的合并承诺——本地多条提交在主线只落一条，推送后工作线快进，沙箱里没被本地改过的路径一起前进。
/// 约束：沙箱里没提交的改动不随推送前进；主线历史只多一条发布提交。
#[test]
pub(crate) fn work_push_squashes_local_commits_and_fast_forwards_the_sandbox() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let base = ws.work_commit_user("w", "base.md", b"b", 1, 0).unwrap();
    let sd = ws.roots("w", &[]).unwrap().store;

    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "one.md", b"1").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["one.md"], &[], "一", 2))
        .unwrap();
    store.write_under(&sb, "two.md", b"2").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["two.md"], &[], "二", 3))
        .unwrap();
    store.write_under(&sb, "dirty.md", b"dirty").unwrap();

    let rep = push(&ws, "w", "a", "发布两个文件", 4, 4).unwrap();
    let published = rep.commit.expect("推送要落主线提交");
    assert_eq!(
        rep.changes.len(),
        2,
        "推送报告只列它带上去的路径：{:?}",
        rep.changes
    );
    let c = store.read_commit(&sd, published).unwrap().unwrap();
    assert_eq!(c.parent, Some(base), "主线只在发布点之后加一条提交");
    let tree = c.tree.clone().expect("发布提交带整棵树");
    assert!(tree.contains_key("one.md") && tree.contains_key("two.md"));
    assert!(!tree.contains_key("dirty.md"), "没提交的改动不上主线");
    assert_eq!(
        ws.work_head("w").unwrap(),
        Some(published),
        "推送后工作线快进到发布点，下次推送才不会把主线内容当成删除"
    );

    // 沙箱两个已发布的文件都还在；未提交的 dirty.md 原样保留。
    assert_eq!(
        store.read_under(&sb, "one.md").unwrap().as_deref(),
        Some(&b"1"[..])
    );
    assert!(store.stat_under(&sb, "dirty.md").unwrap().is_some());
}
/// 目的：work_diff 默认比「工作线头 ↔ 沙箱」：文本给行级 hunk，二进制只报类型不冒充文本。
/// 约束：只读；显式 from/to 与路径过滤都生效。
#[test]
pub(crate) fn work_diff_shows_line_changes_and_never_pretends_binary_is_text() {
    use crate::capabilities::workspace::api::{ChangeKind, DiffRequest, DiffSign};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store
        .write_under(&sb, "note.md", b"one\ntwo\nthree\n")
        .unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap();
    store
        .write_under(&sb, "note.md", b"one\nTWO\nthree\nfour\n")
        .unwrap();
    store.write_under(&sb, "clip.bin", &[0u8, 1, 2]).unwrap();

    let req = DiffRequest {
        from: None,
        to: None,
        paths: Vec::new(),
    };
    let rep = ws.work_diff("w", "a", &req).unwrap();
    assert_eq!(rep.entries.len(), 2, "两条路径不同：{:?}", rep.entries);
    let note = rep.entries.iter().find(|e| e.path == "note.md").unwrap();
    assert_eq!(note.kind, ChangeKind::Modify);
    assert!(!note.binary && !note.too_large, "{:?}", note);
    let signs: Vec<DiffSign> = note
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter().map(|l| l.sign))
        .collect();
    assert!(
        signs.contains(&DiffSign::Remove) && signs.contains(&DiffSign::Add),
        "行级差异要有删有增：{:?}",
        note.hunks
    );
    let clip = rep.entries.iter().find(|e| e.path == "clip.bin").unwrap();
    assert!(clip.binary, "含 0 字节按二进制处理：{:?}", clip);
    assert!(clip.hunks.is_empty(), "二进制不展开 hunk：{:?}", clip);

    let only = DiffRequest {
        from: None,
        to: None,
        paths: vec!["clip.bin".to_string()],
    };
    let filtered = ws.work_diff("w", "a", &only).unwrap();
    assert_eq!(filtered.entries.len(), 1, "{:?}", filtered.entries);
}

/// 目的：work_reset 两种形态——paths 把沙箱恢复成工作线头，to 把工作线移回去并重铺沙箱。
/// 约束：只能退到自己工作线的祖先；覆盖前备份进 lost/。
#[test]
pub(crate) fn work_reset_restores_paths_and_moves_the_work_line() {
    use crate::capabilities::workspace::api::ResetRequest;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "note.md", b"v1\n").unwrap();
    let id1 = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap()
        .id;
    store.write_under(&sb, "note.md", b"v2\n").unwrap();
    let id2 = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap()
        .id;

    // a 拉过、但没在自己工作线提交过的路径：hard 回退不该动它。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "pulled.md", b"from-b\n").unwrap();
    ws.work_commit("w", "b", "w--b", &commit_req(&["pulled.md"], &[], "b", 1))
        .unwrap();
    push(&ws, "w", "b", "b", 1, 1).unwrap();
    ws.work_pull("w", "a", &["pulled.md".to_string()]).unwrap();

    // paths：沙箱里的草稿被换回工作线头（v2），旧内容备份进 lost/。
    store.write_under(&sb, "note.md", b"draft\n").unwrap();
    let rep = ws
        .work_reset(
            "w",
            "a",
            &ResetRequest {
                paths: vec!["note.md".to_string()],
                to: None,
                keep_changes: false,
            },
        )
        .unwrap();
    assert_eq!(rep.restored, vec!["note.md".to_string()]);
    assert!(rep.backed_up >= 1, "覆盖草稿要先备份：{:?}", rep);
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v2\n"[..]),
        "paths 形态把沙箱恢复成工作线头的版本"
    );

    // to：工作线移回 v1，沙箱按 v1 重铺。
    let rep = ws
        .work_reset(
            "w",
            "a",
            &ResetRequest {
                paths: Vec::new(),
                to: Some(id1),
                keep_changes: false,
            },
        )
        .unwrap();
    assert_eq!(rep.moved_to, Some(id1));
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v1\n"[..]),
        "移回提交后沙箱按它重铺"
    );
    assert_eq!(
        store.read_under(&sb, "pulled.md").unwrap().as_deref(),
        Some(&b"from-b\n"[..]),
        "拉过但没提交的文件不在工作线树里，hard 回退不该删它"
    );

    // 现在 v2 不在祖先链上了：再想移过去要拒绝（否则那条提交的来路就断了）。
    assert!(
        ws.work_reset(
            "w",
            "a",
            &ResetRequest {
                paths: Vec::new(),
                to: Some(id2),
                keep_changes: false,
            },
        )
        .is_err(),
        "只能退到自己的工作线祖先"
    );
}

/// 目的：`work_sync` 把用户在产品之外改的磁盘内容记成一条主线提交，不给 external 建工作线。
#[test]
pub(crate) fn work_sync_records_external_edits_on_the_main_line() {
    use crate::capabilities::workspace::api::WorkState;
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let shared = shared_of(&ws, "w");
    let sandbox_a = sandbox_of(&ws, "w", "a");
    let store_dir = ws.roots("w", &[]).unwrap().store;

    // 用户在共享区目录里直接建了文件（产品之外的动作）。
    store.write_under(&shared, "note.md", b"v1\n").unwrap();
    let first = ws
        .work_sync("w", &["note.md".to_string()], 1)
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
    let second = ws
        .work_sync("w", &["note.md".to_string()], 2)
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
