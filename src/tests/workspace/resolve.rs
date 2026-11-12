//! 目的：冲突解决与恢复——显式选边、备份、pending 补完、对象回收。
//! 管：take=local/upstream、lost 备份、pending、discard、GC。
//! 不管：自动提交（`auto.rs`）。
//! 联动：`docs/storage/README.md`。

use super::super::prelude::*;
use super::support::*;

/// 目的：两边都改时，人工合并后用 take=local 推进基线，提交成功且主副本拿到合并结果。
#[test]
pub(crate) fn work_resolve_local_marks_merged_result_and_commit_succeeds() {
    use crate::capabilities::workspace::api::ResolveAction;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"merged").unwrap();
    assert!(ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "b", 3))
        .is_err());

    let rep = ws
        .work_resolve("w", "b", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::Marked);
    let commit = ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "合并", 4))
        .unwrap();
    assert_eq!(commit.changes[0].path, "note.md");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"merged"[..])
    );
}

/// 目的：take=upstream 用主副本覆盖本地；覆盖后与主副本一致，不需要再提交。
#[test]
pub(crate) fn work_resolve_upstream_overwrites_local_and_then_is_clean() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"b-change").unwrap();

    let rep = ws
        .work_resolve("w", "b", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::WroteUpstream);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"a-change"[..])
    );
    let st = ws.work_status("w", "b", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Clean);
    assert!(ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "noop", 3))
        .unwrap_err()
        .contains("没有可提交"));
}

/// 目的：上游删文件时，take=upstream 删本地并清基线；take=local 清基线、让本地成为新增。
#[test]
pub(crate) fn work_resolve_handles_upstream_deletion_by_side() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "gone.txt", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "keep.bin", b"bin1", 2, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let both = ["gone.txt".to_string(), "keep.bin".to_string()];
    ws.work_pull("w", "a", &both).unwrap();
    ws.work_pull("w", "b", &both).unwrap();
    store.write_under(&sb_a, "gone.txt", b"local").unwrap();
    store.write_under(&sb_a, "keep.bin", b"bin-local").unwrap();
    ws.work_commit(
        "w",
        "b",
        "w--b",
        &commit_req(&[], &["gone.txt", "keep.bin"], "上游删", 3),
    )
    .unwrap();
    let pulled = ws.work_pull("w", "a", &both).unwrap();
    assert_eq!(pulled.missing, both.to_vec());

    let gone = ws
        .work_resolve("w", "a", &resolve_req(&["gone.txt"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(gone.entries[0].action, ResolveAction::RemovedLocal);
    assert!(store.read_under(&sb_a, "gone.txt").unwrap().is_none());

    let keep = ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["keep.bin"]))
        .unwrap();
    assert_eq!(keep.entries[0].action, ResolveAction::Cleared);
    assert_eq!(
        store.read_under(&sb_a, "keep.bin").unwrap().as_deref(),
        Some(&b"bin-local"[..]),
        "take=local 不能动本地文件"
    );
    let commit = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["keep.bin"], &[], "保留为新增", 4),
        )
        .unwrap();
    assert_eq!(commit.changes[0].path, "keep.bin");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "keep.bin")
            .unwrap()
            .as_deref(),
        Some(&b"bin-local"[..])
    );
}

/// 目的：未拉取但沙箱有同名文件时状态直说，take=local 后可提交；未跟踪文件单列。
#[test]
pub(crate) fn work_status_exposes_both_present_and_local_only_then_resolve_clears_it() {
    use crate::capabilities::workspace::api::ResolveAction;
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "note.md", b"local").unwrap();
    store.write_under(&sb, "extra.bin", &[0, 1, 2]).unwrap();

    let st = ws.work_status("w", "a", &[]).unwrap();
    let note = st
        .entries
        .iter()
        .find(|e| e.path == "note.md")
        .expect("主副本有的路径要出现");
    assert_eq!(note.state, StatusState::BothPresent);
    assert!(st.local_only.contains(&"extra.bin".to_string()));
    let only = ws
        .work_status("w", "a", &["extra.bin".to_string()])
        .unwrap();
    assert_eq!(only.entries[0].state, StatusState::LocalOnly);

    let rep = ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::Marked);
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "合并", 2))
        .unwrap();
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"local"[..])
    );
}

/// 目的：本地没改过时 take=local 必须拒绝；head token 过期也必须拒绝，且都不留改动。
#[test]
pub(crate) fn work_resolve_local_rejects_unmerged_and_stale_head() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveRequest, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();

    // b 本地仍是 v1：take=local 只会用旧内容覆盖上游，整次拒绝。
    let before = store.read_under(&sb_b, "note.md").unwrap();
    let err = ws
        .work_resolve("w", "b", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap_err();
    assert!(err.contains("没有改过"), "{}", err);
    assert_eq!(store.read_under(&sb_b, "note.md").unwrap(), before);
    assert_eq!(ws.work_head("w").unwrap(), Some(2));

    // 带过期 head 的 resolve 同样拒绝：不能把基于上一版的合并登记到新版。
    let stale = ResolveRequest {
        paths: vec!["note.md".to_string()],
        take: ResolveTake::Local,
        expect_head: Some(1),
    };
    let err = ws.work_resolve("w", "b", &stale).unwrap_err();
    assert!(
        err.contains("主副本已从提交") && err.contains("2"),
        "{}",
        err
    );

    // 真正合并后再 resolve：成功，并记录本次基于的 head。
    store.write_under(&sb_b, "note.md", b"merged").unwrap();
    let rep = ws
        .work_resolve("w", "b", &resolve_local_req(&ws, "w", &["note.md"]))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::Marked);
    assert_eq!(rep.head, Some(2));
}

/// 目的：take=local 在“本地缺失 / 没有基线”时给出显式警告，但仍按既有契约执行。
#[test]
pub(crate) fn work_resolve_local_warns_when_local_missing_or_no_base() {
    use crate::capabilities::workspace::api::ResolveWarningKind;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "other.md", b"o1", 2, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    store.write_under(&sb, "other.md", b"local").unwrap();

    let rep = ws
        .work_resolve(
            "w",
            "a",
            &resolve_local_req(&ws, "w", &["note.md", "other.md"]),
        )
        .unwrap();
    assert_eq!(rep.entries.len(), 2);
    assert!(rep
        .warnings
        .iter()
        .any(|w| w.path == "note.md" && w.kind == ResolveWarningKind::LocalMissing));
    assert!(rep
        .warnings
        .iter()
        .any(|w| w.path == "other.md" && w.kind == ResolveWarningKind::NoBase));
}

/// 目的：take=upstream 覆盖前把本地版本备份到 lost/；work_lost 能列出并按当前 hash 恢复。
#[test]
pub(crate) fn work_resolve_upstream_backs_up_and_work_lost_restores() {
    use crate::capabilities::workspace::api::content_hash;
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"b-change").unwrap();

    let rep = ws
        .work_resolve("w", "b", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::WroteUpstream);
    let backup = rep.entries[0].backup.as_deref().expect("要有备份路径");
    assert_eq!(backup, "lost/b/note.md");
    let backup_root = ws.roots("w", &[]).unwrap().store.join("lost").join("b");
    assert_eq!(
        store
            .read_under(&backup_root, "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"b-change"[..])
    );
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"a-change"[..])
    );

    let lost = ws.work_lost("w", "b", None).unwrap();
    assert_eq!((lost.total, lost.entries.len()), (1, 1));
    let e = &lost.entries[0];
    assert_eq!(e.path, "note.md");
    assert_eq!(e.hash, content_hash(b"b-change"));
    let a_hash = content_hash(b"a-change");
    assert_eq!(e.current_hash.as_deref(), Some(a_hash.as_str()));

    // expect_current 不匹配：拒绝，沙箱不动。
    let err = ws
        .work_restore_lost("w", "b", "note.md", "not-the-current-hash")
        .unwrap_err();
    assert!(err.contains("expect_current"), "{}", err);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"a-change"[..])
    );

    // 按当前 hash 恢复；备份仍保留。
    let current = e.current_hash.clone().unwrap();
    let restored = ws.work_restore_lost("w", "b", "note.md", &current).unwrap();
    assert_eq!(restored.previous.as_deref(), Some(current.as_str()));
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"b-change"[..])
    );
    assert_eq!(ws.work_lost("w", "b", None).unwrap().entries.len(), 1);

    // 当前文件不存在时，expect_current 传空串。
    store.remove_under(&sb_b, "note.md").unwrap();
    let restored = ws.work_restore_lost("w", "b", "note.md", "").unwrap();
    assert!(restored.previous.is_none());
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"b-change"[..])
    );
}

/// 目的：上游删除时 take=upstream 也要先备份本地版本，再删沙箱文件。
#[test]
pub(crate) fn work_resolve_upstream_backs_up_before_removing_local() {
    use crate::capabilities::workspace::api::{ResolveAction, ResolveTake};
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "gone.txt", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["gone.txt".to_string()]).unwrap();
    ws.work_pull("w", "b", &["gone.txt".to_string()]).unwrap();
    store.write_under(&sb_a, "gone.txt", b"local").unwrap();
    ws.work_commit(
        "w",
        "b",
        "w--b",
        &commit_req(&[], &["gone.txt"], "上游删", 2),
    )
    .unwrap();

    let rep = ws
        .work_resolve("w", "a", &resolve_req(&["gone.txt"], ResolveTake::Upstream))
        .unwrap();
    assert_eq!(rep.entries[0].action, ResolveAction::RemovedLocal);
    let backup = rep.entries[0].backup.as_deref().expect("删除前也要备份");
    assert_eq!(backup, "lost/a/gone.txt");
    let backup_root = ws.roots("w", &[]).unwrap().store.join("lost").join("a");
    assert_eq!(
        store
            .read_under(&backup_root, "gone.txt")
            .unwrap()
            .as_deref(),
        Some(&b"local"[..])
    );
    assert!(store.read_under(&sb_a, "gone.txt").unwrap().is_none());
}

/// 目的：备份失败时整次 resolve 中止，不覆盖本地、不动基线。
#[test]
pub(crate) fn work_resolve_aborts_when_backup_fails() {
    use crate::capabilities::workspace::api::ResolveTake;
    use crate::capabilities::workspace::domain::workstore::StatusState;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_a, "note.md", b"a-change").unwrap();
    ws.work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a", 2))
        .unwrap();
    store.write_under(&sb_b, "note.md", b"b-change").unwrap();

    store.fail_times("copy_under_to_under", 1);
    let err = ws
        .work_resolve("w", "b", &resolve_req(&["note.md"], ResolveTake::Upstream))
        .unwrap_err();
    assert!(err.contains("copy_under_to_under"), "{}", err);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(&b"b-change"[..])
    );
    let st = ws.work_status("w", "b", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Conflict);
}

/// 目的：空路径与三方都不存在的路径都被如实拒绝；部分存在时报告 missing。
#[test]
pub(crate) fn work_resolve_refuses_empty_and_reports_missing() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    assert!(ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &[]))
        .is_err());
    assert!(ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["nope"]))
        .is_err());
    let rep = ws
        .work_resolve("w", "a", &resolve_local_req(&ws, "w", &["note.md", "nope"]))
        .unwrap();
    assert_eq!(rep.entries.len(), 1);
    assert_eq!(rep.missing, vec!["nope"]);
}

/// 目的：用户投喂同样走事务；收尾失败后下一次操作恢复提交与主副本。
#[test]
pub(crate) fn abandoned_user_commit_pending_is_recovered() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    store.fail_times("write_commit", 2);
    assert!(ws.work_commit_user("w", "note.md", b"v1", 1, 0).is_err());
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.head.unwrap().id, 1);
    assert!(ws.work_has("w", "note.md"), "恢复后主副本要有文件");
}

/// 目的：提交号只增不复用；丢弃 / 恢复都不重置计数器。
#[test]
pub(crate) fn commit_ids_stay_monotonic_after_discard_and_restore() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let id1 = ws.work_commit_user("w", "a.txt", b"1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "b.txt", b"2", 2, 0).unwrap();
    let id3 = ws.work_commit_user("w", "c.txt", b"3", 3, 0).unwrap();
    assert_eq!((id1, id2, id3), (1, 2, 3));

    ws.work_discard_after("w", Some(id1)).unwrap();
    let id4 = ws.work_commit_user("w", "d.txt", b"4", 4, 0).unwrap();
    assert_eq!(id4, 4, "丢弃最高提交后不能复用 2/3");

    ws.work_restore_point("w", None).unwrap();
    let id5 = ws.work_commit_user("w", "e.txt", b"5", 5, 0).unwrap();
    assert_eq!(id5, 5, "清空共享区也不重置计数器");
}

/// 目的：提交收尾失败留下 pending；下一次 status 先恢复，再给出新 head。
#[test]
pub(crate) fn abandoned_commit_pending_is_recovered_by_next_status() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();

    store.fail_times("write_commit", 2);
    let err = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .unwrap_err();
    assert!(err.contains("恢复"), "{}", err);

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.head.unwrap().id, 2, "status 要先恢复 pending 再读 head");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"v2"[..])
    );
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
    );
}

/// 目的：一次失败后立即重放成功时，提交按成功返回。
#[test]
pub(crate) fn commit_pending_recovers_immediately_when_the_failure_clears() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    store.write_under(&sb, "note.md", b"v2").unwrap();

    store.fail_times("write_commit", 1);
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "v2", 2))
        .expect("一次失败后应立即重放成功");
    assert_eq!(rep.id, 2);
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
    );
}

/// 目的：restore 的 pending 在崩溃后由下一次操作补完，head 与主副本一起到达目标点。
#[test]
pub(crate) fn abandoned_restore_pending_is_recovered_by_next_head_read() {
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let id1 = ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();

    store.fail_times("set_head", 2);
    assert!(ws.work_restore("w", id1).is_err());
    let head = ws.work_head("w").unwrap();
    assert_eq!(head, Some(id1), "恢复 pending 后 head 回到目标点");
    assert_eq!(
        store
            .read_under(&shared_of(&ws, "w"), "note.md")
            .unwrap()
            .as_deref(),
        Some(&b"v1"[..])
    );
}

/// 目的：discard 先物化再删提交；崩溃后不会留下 dangling head，下一次操作补完删除。
#[test]
pub(crate) fn abandoned_discard_pending_keeps_head_valid_and_is_recovered() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let id1 = ws.work_commit_user("w", "a.txt", b"1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "b.txt", b"2", 2, 0).unwrap();
    let id3 = ws.work_commit_user("w", "c.txt", b"3", 3, 0).unwrap();

    store.fail_times("remove_commit", 2);
    assert!(ws.work_discard_after("w", Some(id1)).is_err());

    // 恢复前 head 已经先物化到 id1，不能指向待删的 id2/id3。
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));
    let _ = ws.work_status("w", "a", &[]).unwrap();
    let root = ws.roots("w", &[]).unwrap().store;
    assert_eq!(store.list_commits(&root).unwrap(), vec![id1]);
    assert!(id2 > id1 && id3 > id2);
}

/// 目的：对象回收只删不可达对象；只被 agent 基线引用的对象、以及可达提交树的对象都要保留。
#[test]
pub(crate) fn work_gc_keeps_index_and_commit_objects_and_removes_the_rest() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let root = ws.roots("w", &[]).unwrap().store;
    let id1 = ws.work_commit_user("w", "keep.txt", b"k1", 1, 0).unwrap();
    let id2 = ws.work_commit_user("w", "gone.txt", b"g1", 2, 0).unwrap();
    let id3 = ws.work_commit_user("w", "gone.txt", b"g2", 3, 0).unwrap();
    assert_eq!((id1, id2, id3), (1, 2, 3));
    // a 的基线引用 gone 的 g2；这个对象在丢弃提交后仍必须保留。
    ws.work_pull("w", "a", &["keep.txt".to_string(), "gone.txt".to_string()])
        .unwrap();
    // 一个非指纹杂项文件（模拟原子写崩溃留下的临时文件）。
    store.write_object(&root, ".tmp-stale", b"x").unwrap();
    ws.work_discard_after("w", Some(id1)).unwrap();

    let dry = ws.work_gc("w", false).unwrap();
    assert!(!dry.applied);
    assert_eq!(dry.objects, 3, "k1/g1/g2");
    assert_eq!(dry.reachable_objects, 2, "k1 + 基线引用的 g2");
    assert_eq!(dry.unreachable_objects, 1, "g1 只属于被丢弃的提交");
    assert_eq!(dry.removed_objects, 0);
    assert_eq!(dry.ignored_files, 1);
    assert!(store
        .read_object(
            &root,
            &crate::capabilities::workspace::api::content_hash(b"g1")
        )
        .is_ok());

    let applied = ws.work_gc("w", true).unwrap();
    assert!(applied.applied);
    assert_eq!(applied.removed_objects, 1);
    assert_eq!(applied.removed_ignored, 1);
    assert!(
        store
            .read_object(
                &root,
                &crate::capabilities::workspace::api::content_hash(b"g1")
            )
            .is_err(),
        "g1 应被回收"
    );
    assert!(
        store
            .read_object(
                &root,
                &crate::capabilities::workspace::api::content_hash(b"g2")
            )
            .is_ok(),
        "基线引用的 g2 必须保留"
    );
    // 基线仍能看三方；保留提交仍能物化。
    let v = ws.work_versions("w", "a", "gone.txt").unwrap();
    assert!(
        v.base.is_some(),
        "index-only 对象被删会让 work_versions 失效"
    );
    ws.work_restore("w", id1).unwrap();
}

/// 目的：只读共享区：读得到、写不进；写自己的沙箱照旧；围栏也不含主副本。这是"agent 绕不过 pull/commit 直接写共享区"的工具层证据，围栏层的证据由三平台探针给。
#[test]
pub(crate) fn read_only_shared_refuses_builtin_writes_and_leaves_the_fence() {
    use crate::kernel::api::FenceSpec;
    let sb = test_sandbox_readonly("a", &[]);
    let io = InMemorySysIo::new();
    io.seed(&["demo", "work", "note.txt"], "内容");
    // 进 JSON / 补丁的路径一律用**书写形式**（/ 分隔；Windows 反斜杠在 JSON 里非法）。
    let work = s(&["demo", "work", "note.txt"]);

    // 读仍然可以（默认只读 = 可读不可写）。
    let rd = run_builtin(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", work));
    assert!(rd.ok, "只读不等于读不到：{}", rd.output);

    // 写 / 改 / 补丁一律被工具层拒绝。
    let wr = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", work),
    );
    assert!(!wr.ok && wr.output.contains("只读"), "{}", wr.output);
    let ed = run_builtin(
        &sb,
        &io,
        "edit",
        &format!(
            "{{\"path\":\"{}\",\"old_string\":\"内容\",\"new_string\":\"新\"}}",
            work
        ),
    );
    assert!(!ed.ok && ed.output.contains("只读"), "{}", ed.output);
    let patch = format!(
        "*** Update File: {}\n*** SEARCH\n内容\n*** REPLACE\n新\n*** End File\n",
        work
    );
    let pa = run_builtin(&sb, &io, "patch", &patch);
    assert!(!pa.ok && pa.output.contains("只读"), "{}", pa.output);

    // 自己的沙箱照旧可写。
    let own = s(&["demo", "a", "out.txt"]);
    let ok = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", own),
    );
    assert!(ok.ok, "{}", ok.output);

    // 围栏：主副本永远不进 rw（写共享只能走 work_commit）；私有沙箱进 rw。
    let spec = FenceSpec::for_session(&sb, &[], false);
    assert!(!spec.rw.contains(&sb.shared), "主副本不该进 rw");
    assert!(spec.rw.contains(&sb.private));
    // 会话租约从沙箱带出：同名 agent 的多个并发会话靠它区分围栏归属。
    assert_eq!(spec.lease, sb.session, "围栏租约 = 沙箱的会话 id");
    let writable = test_sandbox("a", &[]);
    let spec2 = FenceSpec::for_session(&writable, &[], false);
    assert!(!spec2.rw.contains(&writable.shared), "共享写不直接进 rw");
}

/// 目的：env.rs 装出来的 **agent 会话沙箱**默认就是只读共享区（生产接线，不是测试助手自己设的）。
#[test]
pub(crate) fn agent_sandboxes_from_a_work_are_read_only_for_the_shared_area() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["[]".into()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let meta = core.history_open(&sid).unwrap().0;
    let roster = core.scan();
    let sandboxes = core.sandboxes(&meta, &roster).unwrap();
    let sb = sandboxes.list.first().expect("至少一个 agent 沙箱");
    let (place, path) = sb.resolve(&p(&["w", "work", "x.txt"])).unwrap();
    assert!(
        !sb.can_write(&place, &path),
        "生产里 agent 沙箱不能直接写共享区"
    );
}

/// 目的：GC 只从引用出发算活集合：有 live 钉子时旧提交保住，引用全删才真正回收。
/// 约束：留档/回档不删提交，删引用才省空间。
#[test]
pub(crate) fn work_gc_reclaims_commits_only_after_refs_drop() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "a.md", b"1", 1, 0).unwrap();
    ws.work_commit_user("w", "b.md", b"2", 2, 0).unwrap();
    let sd = ws.roots("w", &[]).unwrap().store;

    // 回档前把当前线钉成 live/2：旧提交仍可达。
    ws.work_restore("w", 1).unwrap();
    assert!(store.read_ref(&sd, "live/2").unwrap().is_some());
    assert_eq!(ws.work_gc("w", true).unwrap().removed_commits, 0);
    assert_eq!(store.list_commits(&sd).unwrap(), vec![1, 2]);

    // 删掉 live 与 main 两条引用后，两条提交都不可达，GC 才回收。
    store.remove_ref(&sd, "live/2").unwrap();
    store.remove_ref(&sd, "main").unwrap();
    let rep = ws.work_gc("w", true).unwrap();
    assert_eq!(rep.removed_commits, 2);
    assert!(store.list_commits(&sd).unwrap().is_empty());
}
