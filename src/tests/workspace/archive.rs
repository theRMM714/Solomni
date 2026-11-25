//! 目的：留档、恢复与删除——主副本与所有沙箱一起到同一个提交点。
//! 管：物化、恢复档案、删除回退与 lost。
//! 不管：单次提交（`work.rs`）。
//! 联动：`docs/storage/README.md`。

use super::support::*;

/// 目的：留档一次把主副本与每个 agent 的沙箱一起物化到同一个提交点，未提交内容进该 agent 的 lost 可读回。
#[test]
pub(crate) fn work_archive_moves_every_sandbox_to_one_point_and_keeps_uncommitted_in_lost() {
    let (ws, _dirs, store) = work_ws();
    let agents = ["a".to_string(), "b".to_string(), "c".to_string()];
    ws.ensure_layout("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");
    let sb = sandbox_of(&ws, "w", "b");
    let sc = sandbox_of(&ws, "w", "c");

    // 第 2 行：a 提交 v1 并推送，成为唯一主线提交。
    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    ws.work_commit("w", "a", "w--a", &r).unwrap();
    let id1 = push(&ws, "w", "a", "v1", 2, 2).unwrap().commit.unwrap();

    // 三个沙箱各有未提交内容：a 改已提交文件，b / c 各写一份草稿。
    store.write_under(&sa, "note.md", b"a-local").unwrap();
    store.write_under(&sb, "b-draft.md", b"b-draft").unwrap();
    store.write_under(&sc, "c-draft.md", b"c-draft").unwrap();

    let mut keep = std::collections::BTreeMap::new();
    keep.insert(String::new(), 2u64);
    keep.insert("a".to_string(), 2u64);
    keep.insert("b".to_string(), 2u64);
    keep.insert("c".to_string(), 2u64);
    let report = ws.work_archive("w", &keep, &agents, 1).unwrap();
    assert_eq!(report.shared, Some(id1));
    assert_eq!(report.point, Some(id1));
    assert_eq!(report.backed_up.get("a"), Some(&1));
    assert_eq!(report.backed_up.get("b"), Some(&1));
    assert_eq!(report.backed_up.get("c"), Some(&1));

    for (name, sbx) in [("a", &sa), ("b", &sb), ("c", &sc)] {
        assert_eq!(
            store.read_under(sbx, "note.md").unwrap().as_deref(),
            Some(&b"v1"[..]),
            "{} 的沙箱要与留档点同内容",
            name
        );
    }
    assert!(store.read_under(&sb, "b-draft.md").unwrap().is_none());
    assert!(store.read_under(&sc, "c-draft.md").unwrap().is_none());
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );

    // 未提交内容没被丢：按 lost 里的 @archive-<mark>/<路径> 读得回，恢复时放回原路径。
    let lost_b = ws.work_lost("w", "b", None).unwrap();
    assert!(
        lost_b
            .entries
            .iter()
            .any(|e| e.path == "b-draft.md" && e.backup == "@archive-1/b-draft.md"),
        "{:?}",
        lost_b
    );
    let back = ws
        .work_restore_lost("w", "b", "@archive-1/b-draft.md", "")
        .unwrap();
    assert_eq!(back.size, "b-draft".len() as u64);
    assert_eq!(
        store.read_under(&sb, "b-draft.md").unwrap().as_deref(),
        Some(&b"b-draft"[..]),
        "恢复要落回原路径，不能落到带备份前缀的字面路径"
    );
    let lost_a = ws.work_lost("w", "a", None).unwrap();
    assert!(
        lost_a.entries.iter().any(|e| e.path == "note.md"),
        "{:?}",
        lost_a
    );
}

/// 目的：恢复档案让主副本与所有沙箱一起回到留档点，并把留档之后长出来的分叉整条丢弃。
#[test]
pub(crate) fn work_restore_archive_rewinds_every_sandbox_and_drops_the_fork() {
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    let agents = ["a".to_string(), "b".to_string()];
    ws.ensure_layout("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");
    let sb = sandbox_of(&ws, "w", "b");
    let root = ws.roots("w", &[]).unwrap().store;

    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    ws.work_commit("w", "a", "w--a", &r).unwrap();
    let id1 = push(&ws, "w", "a", "v1", 2, 2).unwrap().commit.unwrap();

    let mut keep = std::collections::BTreeMap::new();
    for name in ["", "a", "b"] {
        keep.insert(name.to_string(), 2u64);
    }
    ws.work_archive("w", &keep, &agents, 1).unwrap();

    // 留档之后继续走：a 在第 6 行提交 v2 并推送，b 的沙箱也跟着写成 v2（未提交）。
    store.write_under(&sa, "note.md", b"v2").unwrap();
    let mut r2 = commit_req(&["note.md"], &[], "v2", 2);
    r2.line = 6;
    ws.work_commit("w", "a", "w--a", &r2).unwrap();
    let id2 = push(&ws, "w", "a", "v2", 3, 6).unwrap().commit.unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();
    assert_eq!(ws.work_head("w").unwrap(), Some(id2));

    assert_eq!(ws.work_restore_archive("w", 1).unwrap(), Some(id1));
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));
    for (name, path) in [("主副本", &shared), ("a", &sa), ("b", &sb)] {
        assert_eq!(
            store.read_under(path, "note.md").unwrap().as_deref(),
            Some(&b"v1"[..]),
            "{} 要回到留档点",
            name
        );
    }
    // b 在恢复前的 v2 也留了一份在 lost 里。
    assert!(
        ws.work_lost("w", "b", None)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == "note.md" && e.backup == "@restore-1/note.md"),
        "恢复前的沙箱内容要进 lost"
    );

    // 恢复 = 回到留档操作前：留档之后的推送被丢弃，那条线不再可达，也回滚不回去。
    assert!(
        store.read_commit(&root, id2).unwrap().is_none(),
        "留档之后的推送要被丢弃"
    );
    assert!(
        store
            .read_ref(&root, &format!("live/{}", id2))
            .unwrap()
            .is_none(),
        "丢弃的分叉不留下悬空的 live 引用"
    );
    assert!(
        ws.work_restore("w", id2).is_err(),
        "被丢弃的分叉不能回滚过去"
    );
    assert!(store.read_ref(&root, "archive/1").unwrap().is_none());
    assert!(
        ws.work_restore_archive("w", 1).is_err(),
        "恢复过的档案不能再恢复一次"
    );
}

/// 目的：删除把主副本与所有沙箱一起回退到同一个提交点，未提交内容进 lost，删除点之后的留档一并作废。
#[test]
pub(crate) fn work_delete_rolls_every_sandbox_back_and_prunes_archives() {
    use crate::units::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    let agents = ["a".to_string(), "b".to_string()];
    ws.ensure_layout("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");
    let sb = sandbox_of(&ws, "w", "b");
    let root = ws.roots("w", &[]).unwrap().store;

    // 第 2 行：a 提交 v1 并推送，并在这一点留档。
    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    ws.work_commit("w", "a", "w--a", &r).unwrap();
    let id1 = push(&ws, "w", "a", "v1", 2, 2).unwrap().commit.unwrap();
    let mut keep = std::collections::BTreeMap::new();
    for name in ["", "a", "b"] {
        keep.insert(name.to_string(), 2u64);
    }
    ws.work_archive("w", &keep, &agents, 1).unwrap();

    // 留档之后继续走：第 6 行提交 v2 并推送；两个沙箱各留一份未提交内容。
    store.write_under(&sa, "note.md", b"v2").unwrap();
    let mut r2 = commit_req(&["note.md"], &[], "v2", 2);
    r2.line = 6;
    ws.work_commit("w", "a", "w--a", &r2).unwrap();
    let id2 = push(&ws, "w", "a", "v2", 3, 6).unwrap().commit.unwrap();
    store.write_under(&sa, "note.md", b"a-local").unwrap();
    store.write_under(&sb, "b-draft.md", b"b-draft").unwrap();
    assert_eq!(ws.work_head("w").unwrap(), Some(id2));

    let report = ws.work_delete("w", &keep, &agents, &[]).unwrap();
    assert_eq!(report.point, Some(id1));
    assert_eq!(report.backed_up.get("a"), Some(&1));
    assert_eq!(report.backed_up.get("b"), Some(&1));
    assert_eq!(report.pruned, vec![1]);
    assert_eq!(ws.work_head("w").unwrap(), Some(id1), "删除点之后提交不留");

    for (name, path) in [("主副本", &shared), ("a", &sa), ("b", &sb)] {
        assert_eq!(
            store.read_under(path, "note.md").unwrap().as_deref(),
            Some(&b"v1"[..]),
            "{} 要落到删除点",
            name
        );
    }
    assert!(store.read_under(&sb, "b-draft.md").unwrap().is_none());
    assert!(
        ws.work_lost("w", "a", None)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == "note.md" && e.backup == format!("@delete-{}/note.md", id1)),
        "删除前的沙箱内容要进 lost"
    );
    assert!(
        ws.work_lost("w", "b", None)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == "b-draft.md" && e.backup == format!("@delete-{}/b-draft.md", id1)),
        "删除前的草稿要进 lost"
    );

    // 删除点之后的留档级联作废。
    assert!(store.read_ref(&root, "archive/1").unwrap().is_none());
    assert!(ws.work_archives("w").unwrap().is_empty());
}
/// 目的：留档把沙箱铺回它自己存档线上的快照，而不是主线树；主线树只决定共享区。
#[test]
pub(crate) fn work_archive_puts_each_sandbox_on_its_own_save_snapshot() {
    let (ws, _dirs, store) = work_ws();
    let agents = ["a".to_string()];
    ws.ensure_layout("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");

    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    ws.work_commit("w", "a", "w--a", &r).unwrap();
    push(&ws, "w", "a", "v1", 2, 2).unwrap();

    // 沙箱存档：草稿只在存档线，主线没有它。
    store.write_under(&sa, "draft.md", b"draft").unwrap();
    ws.work_commit_auto("w", "a", "w--a", &commit_req(&["draft.md"], &[], "草稿", 4))
        .unwrap()
        .expect("应当记一次存档");
    // 留档前又写了一份没存档的内容：它会进 lost。
    store.write_under(&sa, "later.md", b"later").unwrap();

    let mut keep = std::collections::BTreeMap::new();
    keep.insert(String::new(), 2u64);
    keep.insert("a".to_string(), 4u64);
    ws.work_archive("w", &keep, &agents, 1).unwrap();

    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..]),
        "共享区落在主线树上"
    );
    assert!(
        store.read_under(&shared, "draft.md").unwrap().is_none(),
        "存档草稿不进共享区"
    );
    assert_eq!(
        store.read_under(&sa, "draft.md").unwrap().as_deref(),
        Some(&b"draft"[..]),
        "沙箱回到自己的存档快照"
    );
    assert!(store.read_under(&sa, "later.md").unwrap().is_none());
    assert!(
        ws.work_lost("w", "a", None)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == "later.md" && e.backup == "@archive-1/later.md"),
        "留档前的未存档内容进 lost"
    );

    // 恢复：带回来的仍是存档快照里的内容，而不是主线树。
    ws.work_restore_archive("w", 1).unwrap();
    assert_eq!(
        store.read_under(&sa, "draft.md").unwrap().as_deref(),
        Some(&b"draft"[..])
    );
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
}
