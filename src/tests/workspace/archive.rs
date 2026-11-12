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
    ws.prepare("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");
    let sb = sandbox_of(&ws, "w", "b");
    let sc = sandbox_of(&ws, "w", "c");

    // 第 2 行：a 提交 v1，成为唯一主线提交。
    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    let id1 = ws.work_commit("w", "a", "w--a", &r).unwrap().id;

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

    // 未提交内容没被丢：按 lost 里的 archive-<mark>/<路径> 读得回。
    let lost_b = ws.work_lost("w", "b", None).unwrap();
    assert!(
        lost_b
            .entries
            .iter()
            .any(|e| e.path == "archive-1/b-draft.md"),
        "{:?}",
        lost_b
    );
    let back = ws
        .work_restore_lost("w", "b", "archive-1/b-draft.md", "")
        .unwrap();
    assert_eq!(back.size, "b-draft".len() as u64);
    assert_eq!(
        store
            .read_under(&sb, "archive-1/b-draft.md")
            .unwrap()
            .as_deref(),
        Some(&b"b-draft"[..]),
        "备份按 lost 里的相对路径放回沙箱"
    );
    assert!(store.read_under(&sb, "b-draft.md").unwrap().is_none());
    let lost_a = ws.work_lost("w", "a", None).unwrap();
    assert!(
        lost_a.entries.iter().any(|e| e.path == "archive-1/note.md"),
        "{:?}",
        lost_a
    );
}

/// 目的：恢复档案让主副本与所有沙箱一起回到留档点；离开的那条线钉进 refs/live，仍可再回滚。
#[test]
pub(crate) fn work_restore_archive_rewinds_every_sandbox_and_pins_the_line_it_left() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    let agents = ["a".to_string(), "b".to_string()];
    ws.prepare("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");
    let sb = sandbox_of(&ws, "w", "b");
    let root = ws.roots("w", &[]).unwrap().store;

    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    let id1 = ws.work_commit("w", "a", "w--a", &r).unwrap().id;

    let mut keep = std::collections::BTreeMap::new();
    for name in ["", "a", "b"] {
        keep.insert(name.to_string(), 2u64);
    }
    ws.work_archive("w", &keep, &agents, 1).unwrap();

    // 留档之后继续走：a 在第 6 行提交 v2，b 的沙箱也跟着写成 v2（未提交）。
    store.write_under(&sa, "note.md", b"v2").unwrap();
    let mut r2 = commit_req(&["note.md"], &[], "v2", 2);
    r2.line = 6;
    let id2 = ws.work_commit("w", "a", "w--a", &r2).unwrap().id;
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
            .any(|e| e.path == "restore-1/note.md"),
        "恢复前的沙箱内容要进 lost"
    );

    // 离开的那条线被钉住（提交不删、可再回滚）；档案引用用完即移除。
    assert_eq!(
        store
            .read_ref(&root, "live/2")
            .unwrap()
            .and_then(|r| r.target),
        Some(id2)
    );
    assert!(store.read_ref(&root, "archive/1").unwrap().is_none());
    assert!(
        ws.work_restore_archive("w", 1).is_err(),
        "恢复过的档案不能再恢复一次"
    );
    ws.work_restore("w", id2).unwrap();
    assert_eq!(ws.work_head("w").unwrap(), Some(id2));
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v2"[..])
    );
}

/// 目的：删除把主副本与所有沙箱一起回退到同一个提交点，未提交内容进 lost，删除点之后的留档一并作废。
#[test]
pub(crate) fn work_delete_rolls_every_sandbox_back_and_prunes_archives() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    let agents = ["a".to_string(), "b".to_string()];
    ws.prepare("w", &agents).unwrap();
    let shared = shared_of(&ws, "w");
    let sa = sandbox_of(&ws, "w", "a");
    let sb = sandbox_of(&ws, "w", "b");
    let root = ws.roots("w", &[]).unwrap().store;

    // 第 2 行：a 提交 v1，并在这一点留档。
    store.write_under(&sa, "note.md", b"v1").unwrap();
    let mut r = commit_req(&["note.md"], &[], "v1", 1);
    r.line = 2;
    let id1 = ws.work_commit("w", "a", "w--a", &r).unwrap().id;
    let mut keep = std::collections::BTreeMap::new();
    for name in ["", "a", "b"] {
        keep.insert(name.to_string(), 2u64);
    }
    ws.work_archive("w", &keep, &agents, 1).unwrap();

    // 留档之后继续走：第 6 行提交 v2；两个沙箱各留一份未提交内容。
    store.write_under(&sa, "note.md", b"v2").unwrap();
    let mut r2 = commit_req(&["note.md"], &[], "v2", 2);
    r2.line = 6;
    let id2 = ws.work_commit("w", "a", "w--a", &r2).unwrap().id;
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
            .any(|e| e.path == "delete-1/note.md"),
        "删除前的沙箱内容要进 lost"
    );
    assert!(
        ws.work_lost("w", "b", None)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == "delete-1/b-draft.md"),
        "删除前的草稿要进 lost"
    );

    // 删除点之后的留档级联作废。
    assert!(store.read_ref(&root, "archive/1").unwrap().is_none());
    assert!(ws.work_archives("w").unwrap().is_empty());
}
