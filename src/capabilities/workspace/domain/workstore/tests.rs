//! 目的：workstore 的纯判定用例——路径校验、三方比较、解决判定与状态映射。
//! 管：`clean_rel` / `expand_paths` / `plan_commit` / `pull_verdict` / `local_take_verdict` / `status_of` / `lost_rel` 的边界。
//! 不管：落盘与 IO（由 `workspace::detail` 与能力面用例覆盖）。
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
    // 没拉取过就删 → 冲突（不是静默删）。
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
fn lost_rel_is_one_clean_agent_component_plus_a_clean_rel() {
    assert_eq!(lost_rel("a", "dir/x.md").unwrap(), "lost/a/dir/x.md");
    assert!(lost_rel("", "x").is_err());
    assert!(lost_rel("..", "x").is_err());
    assert!(lost_rel("a/b", "x").is_err());
    assert!(lost_rel("a\\b", "x").is_err());
    assert!(lost_rel("a", "../x").is_err());
}
