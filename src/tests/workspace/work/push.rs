//! 目的：共享区传送：自动提交、推送、拉取、重置备份找回、差异。
//! 管：squash 与快进、主线路径保留、双方删除、本地改动分离、lost 命名、二进制差异。
//! 不管：状态视图（`work/status.rs`）、冲突解法（`work/sync.rs`）。
//! 联动：`docs/storage/README.md`。

use crate::tests::workspace::support::*;

/// 目的：自动提交是**存档**——它把沙箱改动记进该 agent 的存档线，共享区与主线一个字都不动。
/// 约束：存档线是 `refs/agents/<agent>/saves`；要进共享区仍得显式 work_commit。
#[test]
pub(crate) fn work_commit_auto_saves_the_sandbox_without_touching_the_shared_area() {
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
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
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
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

/// 目的：推送的合并基准是分叉点而不是拉取基线：用户投喂的文件我只拉过、没在工作线提交，不能被当成「我删掉了」。
/// 约束：推送不删主线上我没动过的路径，也不需要我先提交它。
#[test]
pub(crate) fn work_push_keeps_main_paths_it_never_committed() {
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sd = ws.roots("w", &[]).unwrap().store;
    // 用户投喂一份数据：只进主线，不经过任何 agent 的沙箱。
    ws.work_commit_user("w", "data.csv", b"1,2\n", 1, 0)
        .unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    // 拉一次：路径进了我的拉取基线，但我的工作线从没提交过它。
    ws.work_pull("w", "a", &["data.csv".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"n").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "笔记", 2))
        .unwrap();

    let rep = push(&ws, "w", "a", "只有笔记", 3, 3).unwrap();
    let published = rep.commit.expect("推送要落主线提交");
    assert!(
        !rep.changes.iter().any(|c| c.path == "data.csv"),
        "没提交过的路径不该出现在发布改动里：{:?}",
        rep.changes
    );
    let c = store.read_commit(&sd, published).unwrap().unwrap();
    let tree = c.tree.clone().expect("发布提交带整棵树");
    assert!(
        tree.contains_key("data.csv"),
        "主线上的用户数据不能被推送删掉：{:?}",
        tree.keys().collect::<Vec<_>>()
    );
}

/// 目的：两边都有过的路径，我在沙箱里删掉并显式提交后，推送要把它发布成删除。
/// 约束：删除判定用分叉点（它有）对比我的工作线（它没有），基线不参与。
#[test]
pub(crate) fn work_push_publishes_a_delete_both_sides_had() {
    use crate::units::business::versions::api::ChangeKind;
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let shared = shared_of(&ws, "w");
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "note.md", b"v1").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap();
    push(&ws, "w", "a", "v1", 2, 1).unwrap();

    store.remove_under(&sb, "note.md").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 3))
        .unwrap();
    let rep = push(&ws, "w", "a", "删掉", 4, 2).unwrap();
    assert!(
        rep.changes
            .iter()
            .any(|c| c.path == "note.md" && c.kind == ChangeKind::Delete),
        "推送要发布删除：{:?}",
        rep.changes
    );
    assert!(
        store.stat_under(&shared, "note.md").unwrap().is_none(),
        "共享区里的文件要跟着消失"
    );
}

/// 目的：拉取把「本地已改 / 本地已删、上游没动」如实分类，不再混进「无变化」。
/// 约束：分类不影响落盘——本地内容一律不被覆盖。
#[test]
pub(crate) fn work_pull_separates_local_changes_from_no_change() {
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    store.write_under(&sb, "note.md", b"local").unwrap();
    let rep = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(rep.local_modified, vec!["note.md".to_string()], "{:?}", rep);
    assert!(rep.no_change.is_empty(), "本地改过不算无变化：{:?}", rep);
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"local"[..]),
        "本地内容不被覆盖"
    );

    store.remove_under(&sb, "note.md").unwrap();
    let rep = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(rep.local_deleted, vec!["note.md".to_string()], "{:?}", rep);
    assert!(
        store.stat_under(&sb, "note.md").unwrap().is_none(),
        "本地删掉的不被上游拉回来"
    );
}

/// 目的：overwrite 前的备份按 `@<来源>/<原路径>` 落盘，清单报原路径，恢复放回原路径。
/// 约束：expect_current 双向生效（不符就拒绝），恢复不写到带前缀的字面路径。
#[test]
pub(crate) fn work_lost_restores_a_reset_backup_to_the_original_path() {
    use crate::kernel::api::hash::content_hash;
    use crate::units::business::versions::api::ResetRequest;
    use crate::units::business::versions::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "docs/b.txt", b"v1").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["docs/b.txt"], &[], "v1", 1))
        .unwrap();
    store.write_under(&sb, "docs/b.txt", b"draft").unwrap();
    ws.work_reset(
        "w",
        "a",
        &ResetRequest {
            paths: vec!["docs/b.txt".to_string()],
            to: None,
            keep_changes: false,
        },
    )
    .unwrap();

    let lost = ws.work_lost("w", "a", Some("docs/b.txt")).unwrap();
    assert_eq!(lost.entries.len(), 1, "{:?}", lost);
    let e = &lost.entries[0];
    assert_eq!(e.path, "docs/b.txt", "清单报原路径");
    assert_eq!(e.backup, "@reset-paths/docs/b.txt", "备份位置带来源前缀");
    assert_eq!(e.origin.as_deref(), Some("reset-paths"));
    assert_eq!(
        e.current_hash.as_deref(),
        Some(content_hash(b"v1").as_str())
    );

    let err = ws
        .work_restore_lost("w", "a", "docs/b.txt", "bogus")
        .unwrap_err();
    assert!(err.contains("expect_current"), "{}", err);

    let current = e.current_hash.clone().unwrap();
    let back = ws
        .work_restore_lost("w", "a", "@reset-paths/docs/b.txt", &current)
        .unwrap();
    assert_eq!(back.path, "docs/b.txt");
    assert_eq!(back.backup, "@reset-paths/docs/b.txt");
    assert_eq!(
        store.read_under(&sb, "docs/b.txt").unwrap().as_deref(),
        Some(&b"draft"[..]),
        "恢复要落回原路径"
    );
    assert!(
        store
            .stat_under(&sb, "@reset-paths/docs/b.txt")
            .unwrap()
            .is_none(),
        "不能写到带前缀的字面路径"
    );

    // 只有一份时，按原路径也能恢复。
    let back = ws
        .work_restore_lost("w", "a", "docs/b.txt", &content_hash(b"draft"))
        .unwrap();
    assert_eq!(back.path, "docs/b.txt");
}

/// 目的：同一路径有多份备份时，按原路径恢复要点名一份，按备份位置则精确恢复。
/// 约束：只有一份时按原路径直取（见上一条用例）。
#[test]
pub(crate) fn work_lost_asks_to_name_one_when_a_path_has_several_backups() {
    use crate::kernel::api::hash::content_hash;
    use crate::units::business::versions::api::ResetRequest;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let id1 = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v1", 1))
        .unwrap()
        .id;
    store.write_under(&sb, "note.md", b"v2").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap();

    // 第一份：paths 形态把草稿换回工作线头。
    store.write_under(&sb, "note.md", b"draft-1").unwrap();
    ws.work_reset(
        "w",
        "a",
        &ResetRequest {
            paths: vec!["note.md".to_string()],
            to: None,
            keep_changes: false,
        },
    )
    .unwrap();
    // 第二份：to 形态退回 v1，先把 v2 备份。
    store.write_under(&sb, "note.md", b"draft-2").unwrap();
    ws.work_reset(
        "w",
        "a",
        &ResetRequest {
            paths: Vec::new(),
            to: Some(id1),
            keep_changes: false,
        },
    )
    .unwrap();

    let lost = ws.work_lost("w", "a", Some("note.md")).unwrap();
    assert_eq!(lost.entries.len(), 2, "{:?}", lost);
    let backups: Vec<&str> = lost.entries.iter().map(|e| e.backup.as_str()).collect();
    assert!(
        backups.contains(&"@reset-paths/note.md") && backups.contains(&"@reset-1/note.md"),
        "{:?}",
        backups
    );
    assert!(lost.entries.iter().all(|e| e.path == "note.md"));

    let err = ws
        .work_restore_lost("w", "a", "note.md", &content_hash(b"v1"))
        .unwrap_err();
    assert!(err.contains("份备份"), "{}", err);
    assert!(
        err.contains("@reset-paths/note.md") && err.contains("@reset-1/note.md"),
        "{}",
        err
    );

    let back = ws
        .work_restore_lost("w", "a", "@reset-1/note.md", &content_hash(b"v1"))
        .unwrap();
    assert_eq!(back.backup, "@reset-1/note.md");
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"draft-2"[..]),
        "点名的那一份要回到原路径"
    );
}

/// 目的：work_diff 默认比「工作线头 ↔ 沙箱」：文本给行级 hunk，二进制只报类型不冒充文本。
/// 约束：只读；显式 from/to 与路径过滤都生效。
#[test]
pub(crate) fn work_diff_shows_line_changes_and_never_pretends_binary_is_text() {
    use crate::units::business::versions::api::{ChangeKind, DiffRequest, DiffSign};
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
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
