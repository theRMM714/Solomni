//! 目的：workstore 的纯判定用例——路径校验、三方比较、解决判定与状态映射。
//! 管：`clean_rel` / `expand_paths` / `plan_commit` / `plan_merge` / `pull_verdict` / `local_take_verdict` / `status_of` / `work_state_of` / `backup_rel` / `split_backup` 的边界。
//! 不管：落盘与 IO（由 `workspace::detail` 与入站面用例覆盖）。
//! 联动：只经 `super::*` 用纯函数；起作用的源码见同目录 `record.rs` / `path.rs` / `merge.rs` / `resolve.rs` / `status.rs`。

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::*;

fn tree(entries: &[(&str, &str)]) -> Tree {
    entries
        .iter()
        .map(|(p, h)| (p.to_string(), h.to_string()))
        .collect()
}
fn index(entries: &[(&str, &str)]) -> Index {
    entries
        .iter()
        .map(|(p, h)| (p.to_string(), h.to_string()))
        .collect()
}
fn locals(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    entries
        .iter()
        .map(|(p, v)| (p.to_string(), v.map(|s| s.to_string())))
        .collect()
}

#[test]
fn clean_relative_paths_only() {
    assert_eq!(clean_rel("a/b.txt").unwrap(), "a/b.txt");
    for bad in ["", "/abs", "a//b", "a/./b", "a/../b", "a\\b", "a/"] {
        assert!(clean_rel(bad).is_err(), "{}", bad);
    }
}

#[test]
fn folders_expand_and_missing_is_reported() {
    let available: BTreeSet<String> = ["a.md", "dir/x", "dir/sub/y"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (got, missing) = expand_paths(&["dir".to_string(), "a.md".to_string()], &available);
    assert_eq!(got, vec!["a.md", "dir/sub/y", "dir/x"]);
    assert!(missing.is_empty());
    let (_, missing) = expand_paths(&["nope".to_string()], &available);
    assert_eq!(missing, vec!["nope"]);
}

#[test]
fn commit_rules_cover_add_modify_delete_and_noop() {
    // 主副本有 f1（h1），本地是 h2 → modify。
    let plan = plan_commit(
        &index(&[("f1", "h1")]),
        &tree(&[("f1", "h1")]),
        &locals(&[("f1", Some("h2"))]),
    )
    .unwrap();
    assert_eq!(
        plan.changes,
        vec![Change {
            path: "f1".into(),
            kind: ChangeKind::Modify,
            hash: "h2".to_string()
        }]
    );
    // 本地没改、上游已更新 → 没得提交，原因说“上游已更新”。
    let plan = plan_commit(
        &index(&[("f1", "h2")]),
        &tree(&[("f1", "h1")]),
        &locals(&[("f1", Some("h2"))]),
    )
    .unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(
        plan.unchanged,
        vec![("f1".to_string(), Unchanged::UpstreamMoved)]
    );
    // 基线 h1、上游 h2、本地 h3：两边都改了 → 冲突。
    let err = plan_commit(
        &index(&[("f1", "h1")]),
        &tree(&[("f1", "h2")]),
        &locals(&[("f1", Some("h3"))]),
    )
    .unwrap_err();
    assert_eq!(
        err,
        vec![ConflictAt {
            path: "f1".into(),
            kind: Conflict::BothModified
        }]
    );
    // 显式删除：基线 == 上游 → delete；上游动过 → 冲突。
    let plan = plan_commit(
        &index(&[("f1", "h1")]),
        &tree(&[("f1", "h1")]),
        &locals(&[("f1", None)]),
    )
    .unwrap();
    assert_eq!(plan.changes[0].kind, ChangeKind::Delete);
    assert!(
        plan_commit(
            &index(&[("f1", "h1")]),
            &tree(&[("f1", "h9")]),
            &locals(&[("f1", None)])
        )
        .is_err()
    );
    // 上游已删除、本地未改 → 没得提交，原因说“上游已删除”。
    let plan = plan_commit(
        &index(&[("f1", "h1")]),
        &tree(&[]),
        &locals(&[("f1", Some("h1"))]),
    )
    .unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(
        plan.unchanged,
        vec![("f1".to_string(), Unchanged::UpstreamDeleted)]
    );
    // 上游已删除、显式删除也已经是删除状态 → 无需再删。
    let plan = plan_commit(
        &index(&[("f1", "h1")]),
        &tree(&[]),
        &locals(&[("f1", None)]),
    )
    .unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(
        plan.unchanged,
        vec![("f1".to_string(), Unchanged::AlreadyDeleted)]
    );
    // 没基线、主线里也没有这条路径（例如沙箱里删掉一个已自动提交过的文件）→ 已经没有了，不是冲突。
    let plan = plan_commit(&index(&[]), &tree(&[]), &locals(&[("f1", None)])).unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(
        plan.unchanged,
        vec![("f1".to_string(), Unchanged::AlreadyDeleted)]
    );
    // 没拉取过就删、但主线里还有 → 冲突（不是静默删）。
    assert_eq!(
        plan_commit(
            &index(&[]),
            &tree(&[("f1", "h1")]),
            &locals(&[("f1", None)])
        )
        .unwrap_err(),
        vec![ConflictAt {
            path: "f1".into(),
            kind: Conflict::DeleteWithoutBase
        }]
    );
    // 两边都新增同一路径、内容不同 → 冲突。
    assert_eq!(
        plan_commit(
            &index(&[]),
            &tree(&[("f1", "h1")]),
            &locals(&[("f1", Some("h2"))])
        )
        .unwrap_err(),
        vec![ConflictAt {
            path: "f1".into(),
            kind: Conflict::BothAdded
        }]
    );
}

#[test]
fn pull_rules_never_clobber_local_work() {
    assert_eq!(pull_verdict(None, Some("h1"), None), PullVerdict::Update);
    assert_eq!(
        pull_verdict(None, Some("h1"), Some("h9")),
        PullVerdict::Occupied
    );
    assert_eq!(
        pull_verdict(Some("h1"), Some("h1"), Some("h1")),
        PullVerdict::NoChange
    );
    assert_eq!(
        pull_verdict(Some("h1"), Some("h2"), Some("h1")),
        PullVerdict::Update
    );
    assert_eq!(
        pull_verdict(Some("h1"), Some("h2"), Some("h3")),
        PullVerdict::Conflict
    );
    // 上游没动、本地改了/删了：不动本地，但要如实分类，不能算「无变化」。
    assert_eq!(
        pull_verdict(Some("h1"), Some("h1"), Some("h9")),
        PullVerdict::LocalModified
    );
    assert_eq!(
        pull_verdict(Some("h1"), Some("h1"), None),
        PullVerdict::LocalDeleted
    );
    assert_eq!(
        pull_verdict(Some("h1"), None, Some("h1")),
        PullVerdict::Missing
    );
}

#[test]
fn status_separates_local_change_from_upstream_change() {
    assert_eq!(status_of(None, Some("h1"), None), StatusState::NotPulled);
    assert_eq!(
        status_of(Some("h1"), Some("h1"), Some("h1")),
        StatusState::Clean
    );
    assert_eq!(
        status_of(Some("h1"), Some("h1"), Some("h2")),
        StatusState::LocalModified
    );
    assert_eq!(
        status_of(Some("h1"), Some("h1"), None),
        StatusState::LocalDeleted
    );
    assert_eq!(
        status_of(Some("h1"), Some("h2"), None),
        StatusState::Conflict
    );
    assert_eq!(
        status_of(Some("h1"), Some("h2"), Some("h1")),
        StatusState::UpstreamMoved
    );
    assert_eq!(
        status_of(Some("h1"), Some("h2"), Some("h3")),
        StatusState::Conflict
    );
    assert_eq!(
        status_of(None, Some("h1"), Some("h2")),
        StatusState::BothPresent
    );
    assert_eq!(status_of(None, None, Some("h1")), StatusState::LocalOnly);
    assert_eq!(
        status_of(Some("h1"), None, None),
        StatusState::UpstreamDeleted
    );
    assert_eq!(
        status_of(Some("h1"), None, Some("h1")),
        StatusState::UpstreamDeleted
    );
    assert_eq!(
        status_of(Some("h1"), None, Some("h2")),
        StatusState::Conflict
    );
}

/// 目的：工作面五态——工作线头 vs 主线头 vs 沙箱；推送前后分别是未推送与已提交。
#[test]
fn work_state_follows_the_work_line_not_the_main_line() {
    use WorkState::*;
    // 工作线有、沙箱同版：主线同版才是已提交，否则是未推送。
    assert_eq!(work_state_of(Some("w1"), Some("w1"), Some("w1")), Committed);
    assert_eq!(work_state_of(Some("w1"), Some("m1"), Some("w1")), Unpushed);
    assert_eq!(work_state_of(Some("w1"), None, Some("w1")), Unpushed);
    // 提交后又改 / 沙箱删掉。
    assert_eq!(work_state_of(Some("w1"), Some("w1"), Some("w2")), Modified);
    assert_eq!(work_state_of(Some("w1"), Some("w1"), None), Deleted);
    // 没提交过：主线有同版 = 一致；主线有别的版 = 本地改过；主线没有 = 未提交新增。
    assert_eq!(work_state_of(None, Some("m1"), Some("m1")), Synced);
    assert_eq!(work_state_of(None, Some("m1"), Some("m2")), Modified);
    assert_eq!(work_state_of(None, None, Some("l1")), Untracked);
    // 主线有、沙箱没有：还没拉取。
    assert_eq!(work_state_of(None, Some("m1"), None), NotPulled);
}

#[test]
fn local_take_verdict_rejects_unmerged_and_warns_on_blind_states() {
    use LocalTakeVerdict::*;
    assert_eq!(
        local_take_verdict(Some("h1"), Some("h2"), Some("h1")),
        DenyUnchanged
    );
    assert_eq!(local_take_verdict(Some("h1"), Some("h2"), Some("h3")), Ok);
    assert_eq!(
        local_take_verdict(Some("h1"), Some("h1"), Some("h1")),
        Ok,
        "上游没动时 take=local 只是推进基线，不是覆盖"
    );
    assert_eq!(local_take_verdict(None, Some("h2"), None), WarnLocalMissing);
    assert_eq!(
        local_take_verdict(Some("h1"), Some("h2"), None),
        WarnLocalMissing
    );
    assert_eq!(local_take_verdict(None, Some("h2"), Some("h1")), WarnNoBase);
    assert_eq!(
        local_take_verdict(None, Some("h2"), Some("h2")),
        Ok,
        "本地已经等于主副本时无需警告"
    );
    assert_eq!(
        local_take_verdict(Some("h1"), None, Some("h1")),
        Ok,
        "上游已删时保留本地不是覆盖上游"
    );
}

#[test]
fn backup_only_when_local_exists_and_differs_from_head() {
    assert!(needs_backup(Some("h2"), Some("h1")));
    assert!(needs_backup(None, Some("h1")));
    assert!(!needs_backup(Some("h1"), Some("h1")));
    assert!(!needs_backup(Some("h1"), None));
    assert!(!needs_backup(None, None));
}

#[test]
fn backup_location_carries_its_origin_and_split_recognises_old_data() {
    assert_eq!(backup_rel("resolve", "dir/x.md"), "@resolve/dir/x.md");
    assert_eq!(backup_rel("@reset-3", "x.md"), "@reset-3/x.md");
    assert_eq!(
        split_backup("@reset-3/dir/x.md"),
        (Some("reset-3"), "dir/x.md")
    );
    // 旧布局没有标记段：能认出产点前缀的按前缀拆，认不出的整条就是原路径（旧 resolve 备份）。
    assert_eq!(
        split_backup("reset-paths/dir/x.md"),
        (Some("reset-paths"), "dir/x.md")
    );
    assert_eq!(
        split_backup("rewind-delete-4/dir/x.md"),
        (Some("rewind-delete-4"), "dir/x.md")
    );
    assert_eq!(split_backup("docs/b.txt"), (None, "docs/b.txt"));
    assert_eq!(split_backup("@lonely"), (None, "@lonely"));
}

#[test]
fn line_diff_keeps_context_and_reports_both_sides() {
    let hunks = diff_text("a\nb\nc\n", "a\nB\nc\nd\n");
    assert_eq!(hunks.len(), 1, "改动挨得近要并成一段：{:?}", hunks);
    let signs: Vec<DiffSign> = hunks[0].lines.iter().map(|l| l.sign).collect();
    assert!(signs.contains(&DiffSign::Remove), "{:?}", hunks);
    assert!(signs.contains(&DiffSign::Add), "{:?}", hunks);
    assert!(
        hunks[0]
            .lines
            .iter()
            .any(|l| l.sign == DiffSign::Context && l.text == "a"),
        "上下文行要带上：{:?}",
        hunks
    );
    assert_eq!(hunks[0].old_start, 1, "从 1 数：{:?}", hunks);
}

#[test]
fn identical_text_has_no_hunks_and_pure_insert_starts_at_zero() {
    assert!(diff_text("", "").is_empty());
    assert!(diff_text("x\n", "x\n").is_empty());
    let added = diff_text("", "x\n");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].old_len, 0);
    assert_eq!(added[0].old_start, 0);
    assert_eq!(added[0].new_len, 1);
    assert_eq!(added[0].new_start, 1);
    let removed = diff_text("x\n", "");
    assert_eq!(removed[0].new_len, 0);
    assert_eq!(removed[0].new_start, 0);
}
