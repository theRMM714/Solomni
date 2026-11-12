//! 目的：会话树与配置：继续提示、通知行、profile 与身份。
//! 管：continue 提示由状态派生、通知行不进模型对话、根收窄与子不能放宽、创建写入 profile。
//! 不管：单会话创建细节。
//! 联动：`tests::builders`；权限档见 docs/permission/。

use super::super::builders::*;
use super::super::prelude::*;

/// 目的：共享 guest 的子会话也能改配置；写回该 agent 的唯一名单，而不是留一份会漂的副本。
#[test]
pub(crate) fn child_edit_writes_back_to_the_agent_roster() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "collab",
        vec![agent_meta("a", &["a"], None), agent_meta("b", &["b"], None)],
        Default::default(),
    );
    let child = SessionMeta {
        context_freshness: false,
        name: "w--a".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 2,
        agents: vec![agent_meta("a", &["a"], None)],
        exec: Default::default(),
        sandbox: Default::default(),
        parent: Some("w".to_string()),
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    };
    hist.create(&child).expect("子会话落盘");
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b"), module_of("c")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let (prepared, job) = core
        .prepare_edit("w--a", edit_of(vec![("a", &["c"], "")]))
        .expect("子会话可以改模块");
    assert!(job.is_none(), "本机档不需要 guest");
    assert_eq!(prepared.new_meta.agents[0].modules, vec!["c".to_string()]);
    let owner = prepared
        .new_children
        .iter()
        .find(|m| m.name == "w")
        .expect("写回主会话的名单");
    assert_eq!(
        owner.agents[0].modules,
        vec!["c".to_string()],
        "同一 agent 只有一份配置"
    );
    core.commit_edit(prepared).expect("提交改配");
    assert_eq!(
        hist.meta("w").expect("主会话").agents[0].modules,
        vec!["c".to_string()]
    );
    assert_eq!(
        hist.meta("w--a").expect("子会话").agents[0].modules,
        vec!["c".to_string()]
    );
}

/// 目的：树里同一模块被分给两个 agent 时允许（共用），并把落盘名单去重。
#[test]
pub(crate) fn tree_wide_module_overlap_is_allowed_on_child_edit() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "collab",
        vec![agent_meta("a", &["a"], None), agent_meta("b", &["b"], None)],
        Default::default(),
    );
    let child = SessionMeta {
        context_freshness: false,
        name: "w--a".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 2,
        agents: vec![agent_meta("a", &["a"], None)],
        exec: Default::default(),
        sandbox: Default::default(),
        parent: Some("w".to_string()),
        node: None,
        delegation: None,
        thinking: None,
        run: RunState::Active,
        workspace: None,
    };
    hist.create(&child).expect("子会话落盘");
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let prepared = core
        .prepare_edit("w--a", edit_of(vec![("a", &["b"], "")]))
        .expect("同一模块可被多个 agent 共用");
    assert_eq!(prepared.0.new_meta.modules, vec!["b".to_string()]);
    assert_eq!(
        prepared.0.new_children[0].modules,
        vec!["b".to_string()],
        "主会话的扁平清单去重（两个 agent 都用 b 时只留一份）"
    );
}

/// 目的：上下文「继续」只在后端判定有断点时出现——单 agent 末条是用户才有，末条是 AI 没有。
#[test]
pub(crate) fn continue_hint_is_derived_from_the_session_state() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], None)],
        Default::default(),
    );
    hist.append(
        "w",
        &[serde_json::json!({
            "type": "transcript",
            "lines": [{ "id": 0, "kind": "user", "speaker": "用户", "line": "做一下" }],
        })],
    )
    .expect("写入用户行");
    let mut core = core_with_pkgs(
        vec![module_of("a")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let prepared = core.prepare_activation("w").expect("准备启用");
    let view = core.commit_activation(prepared).expect("提交启用");
    let hint = view.continue_action.expect("末条是用户：要有继续上一轮");
    assert_eq!(hint.kind, "turn");
    assert!(hint.label.contains("继续"));
}

/// 目的：内容类系统提示（notice）占一行、自己成一条回复，但**不进模型上下文**。
/// 约束：同一段转录加一条 notice，重建出的对话条数与角色序列与不加时完全一致。
#[test]
pub(crate) fn notice_lines_stay_out_of_the_model_dialogue() {
    use crate::capabilities::conductor::service::replay_dialogue;
    use crate::capabilities::llm::api::ToolMode;
    let events = |with_notice: bool| {
        let mut lines =
            vec![serde_json::json!({"id": 0, "kind": "user", "speaker": "用户", "line": "问一"})];
        if with_notice {
            lines.push(serde_json::json!({
                "id": 1,
                "kind": "notice",
                "reply": 1,
                "line": "[建组] a",
            }));
        }
        let reply = if with_notice { 2 } else { 1 };
        lines.push(serde_json::json!({
            "id": reply,
            "kind": "msg",
            "speaker": "a",
            "verb": "say",
            "line": "答一",
            "reply": reply,
        }));
        vec![serde_json::json!({"type": "transcript", "lines": lines})]
    };
    let (plain, plain_marks, plain_reply, _, _, _) =
        replay_dialogue(&events(false), ToolMode::Envelope);
    let (shown, marks, line_reply, _, _, _) = replay_dialogue(&events(true), ToolMode::Envelope);
    assert_eq!(shown.len(), plain.len(), "提示不进对话");
    assert_eq!(
        shown.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
        plain.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
        "对话的角色序列不变"
    );
    assert_eq!(marks.len(), 3, "三条行各占一份 marks");
    assert_eq!(line_reply, vec![0, 1, 2], "提示行自己成一条回复");
    assert_eq!(plain_marks.len(), 2);
    assert_eq!(plain_reply, vec![0, 1]);
}

/// 目的：根编辑不覆盖子收窄；子只能再收窄，不能换更高档。
#[test]
pub(crate) fn root_edit_keeps_child_narrowing_and_child_cannot_widen() {
    use crate::capabilities::permission::api::{
        Decision, PermissionLayer, ProfileId, ScopeMode, SharedEdit,
    };
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let parent = core
        .create_work(collab_work("p", &["a"], false, "需求"))
        .unwrap()
        .sid;
    let child = core.spawn_agent_session(&parent, "a").unwrap();
    let with = |mut e: SessionEdit, p: PermissionLayer| {
        e.permission = Some(p);
        e
    };
    core.edit_session(
        &child,
        with(
            edit_of(vec![("a", &["a"], "")]),
            PermissionLayer {
                profile: Some(ProfileId::View),
                shared: Some(SharedEdit {
                    read: Some(ScopeMode::Paths(vec!["src".to_string()])),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
    )
    .expect("子收窄");
    core.edit_session(
        &parent,
        with(
            edit_of(vec![("a", &["a"], "")]),
            PermissionLayer {
                shared: Some(SharedEdit {
                    write: Some(ScopeMode::Paths(vec!["docs".to_string()])),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
    )
    .expect("根编辑");
    let (cm, _) = core.history_open(&child).unwrap();
    assert!(cm.sandbox.narrowing.is_some(), "根编辑不能清掉子收窄");
    let (pm, _) = core.history_open(&parent).unwrap();
    assert!(pm.sandbox.ceiling.is_some(), "根 ceiling 落盘");
    let cfg = core.session_config(&child).unwrap();
    assert_eq!(
        cfg.permission.profile,
        ProfileId::View,
        "子档位仍是收窄后的 view"
    );
    assert_eq!(
        cfg.permission.decisions.write,
        Decision::Deny,
        "view 的写决定生效"
    );
    let widen = with(
        edit_of(vec![("a", &["a"], "")]),
        PermissionLayer {
            profile: Some(ProfileId::AllReach),
            ..Default::default()
        },
    );
    let err = core.edit_session(&child, widen).unwrap_err();
    assert!(err.contains("更宽"), "{}", err);
}

/// 目的：创建时选的档位直接落进根 ceiling；旧 meta 不兼容是预期。
#[test]
pub(crate) fn creation_writes_the_chosen_profile_into_the_root_ceiling() {
    use crate::capabilities::permission::api::ProfileId;
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let mut spec = work("wv", WorkMode::Single, &["a"]);
    spec.permission = Some(ProfileId::View);
    let sid = core.create_work(spec).unwrap().sid;
    let (meta, _) = core.history_open(&sid).unwrap();
    assert_eq!(
        meta.sandbox.ceiling.as_ref().and_then(|l| l.profile),
        Some(ProfileId::View)
    );
    assert_eq!(
        core.session_config(&sid).unwrap().permission.profile,
        ProfileId::View
    );
}

/// 目的：快速切档是根级动作——更新根 ceiling，子会话只能再收窄。
#[test]
pub(crate) fn quick_profile_switch_updates_root_and_children_cannot_widen() {
    use crate::capabilities::permission::api::ProfileId;
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let parent = core
        .create_work(collab_work("p", &["a"], false, "需求"))
        .unwrap()
        .sid;
    let child = core.spawn_agent_session(&parent, "a").unwrap();
    core.set_permission_profile(&parent, ProfileId::View)
        .expect("根快速切档");
    let (pm, _) = core.history_open(&parent).unwrap();
    assert_eq!(
        pm.sandbox.ceiling.as_ref().and_then(|l| l.profile),
        Some(ProfileId::View)
    );
    assert_eq!(
        core.session_config(&child).unwrap().permission.profile,
        ProfileId::View,
        "子会话继承新的根 ceiling"
    );
    let err = core
        .set_permission_profile(&child, ProfileId::AllReach)
        .unwrap_err();
    assert!(err.contains("更宽"), "{}", err);
}
