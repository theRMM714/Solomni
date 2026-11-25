//! 目的：共享区重置：按提交点还原路径并移动工作线、回执形式。
//! 管：重置落盘、工作线移动、回执按实际形式命名。
//! 不管：重置后的冲突解法（`work/sync.rs`）、备份找回（`work/push.rs`）。
//! 联动：`docs/storage/README.md`。

use crate::tests::workspace::support::*;

/// 目的：work_reset 两种形态——paths 把沙箱恢复成工作线头，to 把工作线移回去并重铺沙箱。
/// 约束：只能退到自己工作线的祖先；覆盖前备份进 lost/。
#[test]
pub(crate) fn work_reset_restores_paths_and_moves_the_work_line() {
    use crate::units::workspace::api::ResetRequest;
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string(), "b".to_string()])
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
        (rep.before, rep.after),
        (Some(id2), Some(id2)),
        "paths 形态不动工作线：前后是同一个头"
    );
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
    assert_eq!(
        (rep.before, rep.after),
        (Some(id2), Some(id1)),
        "to 形态把工作线从 v2 移回 v1"
    );
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

/// 目的：work_reset 的回执按事实说清「工作线动没动」——paths 形态不动线，不能套用 to 形态的话术。
#[test]
pub(crate) fn work_reset_receipts_name_the_form_that_ran() {
    use crate::units::workspace::api::{ResetRequest, render_reset};
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
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

    // paths：只把沙箱恢复成工作线头，工作线一点都不动。
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
    let text = render_reset(&rep);
    assert!(text.contains("工作线没动"), "{text}");
    assert!(
        !text.contains("工作线已移到"),
        "paths 形态不能说线被移走：{text}"
    );

    // to：这次工作线真的动了——回执要说清从哪个提交移到哪个。
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
    let text = render_reset(&rep);
    assert!(
        text.contains(&format!("工作线从提交 {id2} 移到 {id1}")),
        "{text}"
    );
}
