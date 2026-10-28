//! 权限能力（permission）的用例：四档、reach 求交、共享区三态、落点与提交/拉取作用域。
//!
//! 归属：这些用例钉的是**权限规则本身**（T1），以及内置工具/工作区提交两个执行点对它的消费（T1/T2）。
//! 平台围栏的真机验收在 tests/<平台>/（T4）。

use crate::capabilities::permission::api::{
    validate_defaults, validate_entry, CallScope, Decision, DecisionEdit, Effect,
    PermissionDefaults, PermissionLayer, PermissionSet, ProfileId, Reach, ReachSet, ScopeMode,
    SharedEdit,
};
use crate::capabilities::workspace::domain::workspace::Place;
use crate::tests::doubles::{
    abs, test_sandbox, test_workspace, test_workspace_store, InMemoryWorkStore, InMemoryWorkspace,
};
fn paths(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn pset(profile: ProfileId) -> PermissionSet {
    PermissionDefaults {
        profile,
        ..Default::default()
    }
    .to_permission_set()
}

/// 四档的 reach 与 decision 必须和产品口径一致。
#[test]
fn profiles_carry_the_expected_reach_and_decisions() {
    let view = ProfileId::View.spec();
    assert!(view.reach.contains(Reach::Private));
    assert!(view.reach.contains(Reach::SharedRead));
    assert!(view.reach.contains(Reach::ModuleRead));
    assert!(!view.reach.contains(Reach::SharedWrite));
    assert_eq!(view.decisions.write, Decision::Deny);
    assert_eq!(view.decisions.exec_sandbox, Decision::Deny);

    let workspace = ProfileId::Workspace.spec();
    assert!(workspace.reach.contains(Reach::SharedWrite));
    assert!(workspace.reach.contains(Reach::ModuleWrite));
    assert!(!workspace.reach.contains(Reach::HostRead));
    assert!(!workspace.reach.contains(Reach::Network));
    assert_eq!(workspace.decisions.exec_sandbox, Decision::Allow);
    assert_eq!(workspace.decisions.exec_external, Decision::Deny);

    let external = ProfileId::External.spec();
    assert!(external.reach.contains(Reach::HostRead));
    assert!(external.reach.contains(Reach::Network));
    assert_eq!(external.decisions.exec_external, Decision::Ask);

    let all = ProfileId::AllReach.spec();
    assert_eq!(all.reach, external.reach);
    assert_eq!(all.decisions.exec_external, Decision::Allow);
}

/// reach 的求交与子集是收窄判定的基础。
#[test]
fn reach_intersection_and_subset_are_component_safe() {
    let a = ReachSet::from([Reach::Private, Reach::SharedRead, Reach::Network]);
    let b = ReachSet::from([Reach::Private, Reach::SharedRead]);
    assert_eq!(a.intersection(&b), b);
    assert!(b.is_subset(&a));
    assert!(!a.is_subset(&b));
    assert!(ReachSet::empty().is_subset(&a));
}

/// 共享区范围三态：all / none / paths；空数组不再兼作整棵。
#[test]
fn shared_scope_is_explicitly_tristate() {
    let all = ScopeMode::All;
    let none = ScopeMode::None;
    let src = ScopeMode::Paths(paths(&["src"]));
    assert!(all.allows("任何/路径"));
    assert!(!none.allows("src/a"));
    assert!(src.allows("src/a"));
    assert!(!src.allows("website/a"));
    assert_eq!(all.intersect(&src), src);
    assert_eq!(none.intersect(&src), ScopeMode::None);
    assert_eq!(
        src.intersect(&ScopeMode::Paths(paths(&["src/foo"]))),
        ScopeMode::Paths(paths(&["src/foo"]))
    );
    assert_eq!(
        src.intersect(&ScopeMode::Paths(paths(&["docs"]))),
        ScopeMode::None
    );
    assert!(ScopeMode::Paths(paths(&["src/foo"])).is_subset_of(&src));
    assert!(!src.is_subset_of(&ScopeMode::Paths(paths(&["src/foo"]))));
    assert!(none.is_subset_of(&src));
    assert!(!all.is_subset_of(&src));
}

/// 落点规则：共享区受 reach 与路径范围；私有沙箱不受路径范围，但受 profile 写决定；模块目录只读。
#[test]
fn sandbox_places_follow_the_permission_set() {
    let mut sb = test_sandbox("a1", &["data"]);
    let shared = sb.shared.clone();
    let private = sb.private.clone();
    let module = sb.modules.get("data").expect("模块目录").clone();
    sb.permissions = pset(ProfileId::Workspace);
    assert!(sb.can_read(&Place::Shared, &shared.join("docs").join("x")));
    assert!(!sb.can_write(&Place::Shared, &shared.join("docs").join("x")));
    assert!(sb.can_read(&Place::Private, &private.join("x")));
    assert!(sb.can_write(&Place::Private, &private.join("x")));
    assert!(sb.can_read(
        &Place::Module("data".to_string()),
        &module.join("script.py")
    ));
    assert!(!sb.can_write(
        &Place::Module("data".to_string()),
        &module.join("userdata").join("state.json")
    ));

    // 共享区路径范围：白名单外读写都拒。
    sb.permissions.shared_read = ScopeMode::Paths(paths(&["docs"]));
    assert!(!sb.can_read(&Place::Shared, &shared.join("src").join("x")));
    assert!(sb.can_read(&Place::Shared, &shared.join("docs").join("x")));

    // 仅可查看：私有沙箱可读不可写。
    sb.permissions = pset(ProfileId::View);
    assert!(sb.can_read(&Place::Private, &private.join("x")));
    assert!(!sb.can_write(&Place::Private, &private.join("x")));
}

/// 围栏派生：共享区永远不进 rw；私有沙箱进 rw；模块目录只读、userdata 只给模块写 reach。
#[test]
fn fence_spec_scopes_module_dirs_and_userdata() {
    use crate::kernel::api::FenceSpec;
    let mut sb = test_sandbox("a1", &[]);
    let module = abs(&["mods", "data"]);
    sb.modules.insert("data".to_string(), module.clone());
    sb.modules_with_userdata.insert("data".to_string());
    sb.permissions = pset(ProfileId::Workspace);

    let session = FenceSpec::for_session(&sb, &[], false);
    assert!(session.ro_tree.contains(&module), "模块目录对 agent 只读");
    assert!(
        session.rw.contains(&module.join("userdata")),
        "模块工具可写自己的 userdata"
    );
    assert!(!session.rw.contains(&module), "agent 不能整块写模块目录");
    assert!(!session.rw.contains(&sb.shared), "共享区永不被直接写");
    assert!(session.rw.contains(&sb.private), "私有沙箱可写");
    assert_eq!(session.private, sb.private, "HOME/TEMP 落在私有沙箱");

    // 一次调用：只有声明 module-write 才给 userdata；只有 shared-read 才给共享区只读。
    let write = ReachSet::from([Reach::Private, Reach::ModuleWrite]);
    let call = FenceSpec::for_call(&sb, &write, Some("data"), &session);
    assert!(call.rw.contains(&module.join("userdata")));
    assert!(!call.ro_tree.contains(&sb.shared));
    let no_write = ReachSet::from([Reach::Private, Reach::ModuleRead]);
    let call = FenceSpec::for_call(&sb, &no_write, Some("data"), &session);
    assert!(!call.rw.contains(&module.join("userdata")));
    assert!(call.ro_tree.contains(&module));

    let read = ReachSet::from([Reach::Private, Reach::SharedRead]);
    let call = FenceSpec::for_call(&sb, &read, None, &session);
    assert!(call.ro_tree.contains(&sb.shared));
    assert!(!call.net, "workspace 没有 network reach");
}

/// 提交与拉取的作用域判定（工作区工具消费的那一份纯函数）：越界整条拒绝，不带半次提交。
#[test]
fn work_commit_and_pull_are_scoped() {
    use crate::capabilities::conductor::service::work_tools::{check_readable, check_writable};
    let mut p = pset(ProfileId::Workspace);
    p.shared_read = ScopeMode::Paths(paths(&["docs"]));
    p.shared_write = ScopeMode::Paths(paths(&["docs"]));
    p.deny = paths(&["docs/secret"]);
    assert!(check_readable(&p, &paths(&["docs/a"])).is_ok());
    assert!(check_readable(&p, &paths(&["src/a"])).is_err());
    assert!(check_writable(&p, &paths(&["docs/a"]), &[]).is_ok());
    assert!(check_writable(&p, &paths(&["docs/secret/key"]), &[]).is_err());
    assert!(check_writable(&p, &[], &paths(&["src/a"])).is_err());
    assert!(check_writable(&p, &paths(&["docs/a", "src/b"]), &[]).is_err());

    let view = pset(ProfileId::View);
    assert!(
        check_writable(&view, &paths(&["docs/a"]), &[]).is_err(),
        "仅可查看没有提交权限"
    );
}

/// 目的：work_status 的路径清单按读作用域过滤，显式越界路径直接拒绝。
#[test]
fn work_status_filters_paths_outside_read_scope() {
    use crate::capabilities::conductor::service::work_tools::WorkHandler;
    use crate::capabilities::workspace::api::CommitRequest;
    use crate::kernel::ports::{ToolCtx, ToolHandler};

    let dirs = std::sync::Arc::new(InMemoryWorkspace::new());
    let store = std::sync::Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs, store.clone());
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    let sb_b = ws
        .roots("w", &["b".to_string()])
        .unwrap()
        .agents
        .get("b")
        .cloned()
        .unwrap();
    store.write_under(&sb_b, "public.txt", b"p").unwrap();
    store.write_under(&sb_b, "secret.txt", b"s").unwrap();
    let req = CommitRequest {
        paths: vec!["public.txt".to_string(), "secret.txt".to_string()],
        deletes: Vec::new(),
        message: "提交".to_string(),
        time: 1,
        line: 1,
    };
    ws.work_commit("w", "b", "w--b", &req).unwrap();
    ws.work_pull(
        "w",
        "a",
        &["public.txt".to_string(), "secret.txt".to_string()],
    )
    .unwrap();
    let sb_a = ws
        .roots("w", &["a".to_string()])
        .unwrap()
        .agents
        .get("a")
        .cloned()
        .unwrap();
    store.write_under(&sb_a, "extra-secret.txt", b"x").unwrap();

    let mut p = pset(ProfileId::Workspace);
    p.shared_read = ScopeMode::Paths(paths(&["public.txt"]));
    let h = WorkHandler::new(ws, "w", "a", p);
    let ctx = ToolCtx {
        work: "w",
        agent: "a",
        line: 1,
    };
    let out = h.run(&ctx, "work_status", "{}");
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("public.txt"), "{}", out.output);
    assert!(!out.output.contains("secret.txt"), "{}", out.output);
    assert!(!out.output.contains("extra-secret.txt"), "{}", out.output);

    let denied = h.run(&ctx, "work_status", r#"{"paths":["secret.txt"]}"#);
    assert!(
        !denied.ok && denied.output.contains("读取被拒"),
        "{}",
        denied.output
    );
}

/// 目的：work_lost 的清单与恢复也按读作用域收口，不能拿备份绕过白名单。
#[test]
fn work_lost_is_scoped_by_read_permission() {
    use crate::capabilities::conductor::service::work_tools::WorkHandler;
    use crate::kernel::ports::{ToolCtx, ToolHandler};

    let dirs = std::sync::Arc::new(InMemoryWorkspace::new());
    let store = std::sync::Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs, store.clone());
    ws.prepare("w", &["a".to_string()]).unwrap();
    let root = ws.roots("w", &[]).unwrap().store.join("lost").join("a");
    store
        .write_under(&root, "public/lost.txt", b"public-lost")
        .unwrap();
    store
        .write_under(&root, "secret/lost.txt", b"secret-lost")
        .unwrap();

    let mut p = pset(ProfileId::Workspace);
    p.shared_read = ScopeMode::Paths(paths(&["public"]));
    let h = WorkHandler::new(ws, "w", "a", p);
    let ctx = ToolCtx {
        work: "w",
        agent: "a",
        line: 1,
    };
    let out = h.run(&ctx, "work_lost", "{}");
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("public/lost.txt"), "{}", out.output);
    assert!(!out.output.contains("secret/lost.txt"), "{}", out.output);

    let denied = h.run(&ctx, "work_lost", r#"{"path":"secret/lost.txt"}"#);
    assert!(
        !denied.ok && denied.output.contains("读取被拒"),
        "{}",
        denied.output
    );
    let restore = h.run(
        &ctx,
        "work_lost",
        r#"{"path":"secret/lost.txt","restore":true,"expect_current":""}"#,
    );
    assert!(
        !restore.ok && restore.output.contains("读取被拒"),
        "{}",
        restore.output
    );
}

/// 目的：work_versions / work_resolve 也按读作用域收口，不能拿它们绕过白名单。
#[test]
fn work_versions_and_resolve_are_scoped_by_read_permission() {
    use crate::capabilities::conductor::service::work_tools::WorkHandler;
    use crate::kernel::ports::{ToolCtx, ToolHandler};
    let ws = test_workspace(std::sync::Arc::new(InMemoryWorkspace::new()));
    let mut p = pset(ProfileId::Workspace);
    p.shared_read = ScopeMode::Paths(paths(&["docs"]));
    let h = WorkHandler::new(ws, "w", "a", p);
    let ctx = ToolCtx {
        work: "w",
        agent: "a",
        line: 1,
    };
    let v = h.run(&ctx, "work_versions", r#"{"path":"secret/x"}"#);
    assert!(!v.ok && v.output.contains("读取被拒"), "{}", v.output);
    let r = h.run(
        &ctx,
        "work_resolve",
        r#"{"paths":["secret/x"],"take":"local"}"#,
    );
    assert!(!r.ok && r.output.contains("读取被拒"), "{}", r.output);

    let bad_take = h.run(
        &ctx,
        "work_resolve",
        r#"{"paths":["docs/a"],"take":"nope"}"#,
    );
    assert!(!bad_take.ok && bad_take.output.contains("upstream 或 local"));
    let no_path = h.run(&ctx, "work_versions", "{}");
    assert!(!no_path.ok && no_path.output.contains("path"));
}

/// 根 ceiling + 子 narrowing：子只能收窄；空集合是显式无权限。
#[test]
fn root_ceiling_and_child_narrowing_intersect() {
    let base = pset(ProfileId::Workspace);
    let root = base
        .apply_root(&PermissionLayer {
            shared: Some(SharedEdit {
                read: Some(ScopeMode::All),
                write: Some(ScopeMode::Paths(paths(&["src"]))),
                deny: Vec::new(),
            }),
            ..Default::default()
        })
        .expect("根 ceiling");
    let child = root
        .apply_narrowing(&PermissionLayer {
            profile: Some(ProfileId::View),
            shared: Some(SharedEdit {
                read: Some(ScopeMode::Paths(paths(&["src/foo"]))),
                write: Some(ScopeMode::None),
                deny: Vec::new(),
            }),
            ..Default::default()
        })
        .expect("子 narrowing");
    assert_eq!(child.profile, ProfileId::View);
    assert!(child.read_ok("src/foo/a"));
    assert!(!child.read_ok("src/bar/a"));
    assert!(!child.write_ok("src/foo/a"));
    assert_eq!(child.decisions.write, Decision::Deny);

    // 子不能换更高档，也不能把写范围放宽。
    assert!(root
        .apply_narrowing(&PermissionLayer {
            profile: Some(ProfileId::AllReach),
            ..Default::default()
        })
        .is_err());
    assert!(root
        .apply_narrowing(&PermissionLayer {
            shared: Some(SharedEdit {
                write: Some(ScopeMode::Paths(paths(&["other"]))),
                ..Default::default()
            }),
            ..Default::default()
        })
        .is_err());

    // 显式无权限与“不覆盖”必须分开。
    let no_read = root
        .apply_narrowing(&PermissionLayer {
            shared: Some(SharedEdit {
                read: Some(ScopeMode::None),
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("显式无读");
    assert!(!no_read.read_ok("src/a"));
    assert_eq!(no_read.shared_read, ScopeMode::None);

    // 根与子无交集：求交结果就是显式无权限。
    assert_eq!(
        ScopeMode::Paths(paths(&["src"])).intersect(&ScopeMode::Paths(paths(&["docs"]))),
        ScopeMode::None
    );
}

/// 决定覆盖：子只能更严，根可以显式设置。
#[test]
fn decisions_can_only_tighten_for_children() {
    let base = pset(ProfileId::Workspace);
    let root = base
        .apply_root(&PermissionLayer {
            decisions: Some(DecisionEdit {
                write: Some(Decision::Ask),
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("根决定");
    assert_eq!(root.decisions.write, Decision::Ask);
    assert!(
        root.apply_narrowing(&PermissionLayer {
            decisions: Some(DecisionEdit {
                write: Some(Decision::Allow),
                ..Default::default()
            }),
            ..Default::default()
        })
        .is_err(),
        "子不能放宽"
    );
    let child = root
        .apply_narrowing(&PermissionLayer {
            decisions: Some(DecisionEdit {
                write: Some(Decision::Deny),
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("子收紧");
    assert_eq!(child.decisions.write, Decision::Deny);
}

/// 条目必须是干净的相对路径：绝对路径、上级段、空段、盘符一律拒收。
#[test]
fn entries_must_be_clean_relative_paths() {
    let ok = |e: &str| {
        validate_defaults(&PermissionDefaults {
            shared: crate::capabilities::permission::api::SharedScope {
                read: ScopeMode::Paths(paths(&[e])),
                ..Default::default()
            },
            ..Default::default()
        })
        .is_ok()
    };
    assert!(ok("docs/a"));
    assert!(ok("."));
    for bad in ["/abs/x", "..", "a/../b", "a//b", r"C:\win", ""] {
        assert!(!ok(bad), "应当拒绝：{:?}", bad);
    }
    assert!(validate_entry("../escape").is_err());
}

/// 工具声明的默认 decision 不压过 profile：shell 默认 ask，但工作区内修改的 sandbox scope 仍是 allow。
#[test]
fn profile_decision_overrides_tool_default() {
    let p = pset(ProfileId::Workspace);
    assert_eq!(
        p.decision_for("shell", Effect::Exec, CallScope::Sandbox, Decision::Ask),
        Decision::Allow
    );
    let all = pset(ProfileId::AllReach);
    assert_eq!(
        all.decision_for("shell", Effect::Exec, CallScope::External, Decision::Ask),
        Decision::Allow
    );
    let external = pset(ProfileId::External);
    assert_eq!(
        external.decision_for("shell", Effect::Exec, CallScope::External, Decision::Allow),
        Decision::Ask
    );
}

/// 调用闸门：reach 不够 / scope 越界拒绝；allow_turn 只抑制 ask，不改 reach。
#[test]
fn call_grant_checks_scope_and_reach() {
    let p = pset(ProfileId::Workspace);
    let shell_max = ReachSet::from([
        Reach::Private,
        Reach::SharedRead,
        Reach::ModuleRead,
        Reach::HostRead,
        Reach::Network,
    ]);
    let sandbox = CallScope::Sandbox.reach();
    let granted = p.grant(&shell_max, Some(&sandbox)).expect("sandbox 合法");
    assert_eq!(granted, ReachSet::from([Reach::Private]));
    assert!(
        p.grant(&shell_max, Some(&CallScope::External.reach()))
            .is_err(),
        "workspace 不能请求 external"
    );
    let all = pset(ProfileId::AllReach);
    let granted = all
        .grant(&shell_max, Some(&CallScope::External.reach()))
        .expect("all_reach 可请求 external");
    assert!(granted.contains(Reach::HostRead));
    assert!(granted.contains(Reach::Network));
    assert!(!granted.contains(Reach::SharedWrite));
    assert!(!granted.contains(Reach::ModuleWrite));
}

/// 工具层提问端口**并入裁决通道**（端到端一条）：发起方（工具执行层）自己推问题 + 选项集 →
/// 卡进**同一条队**（gate = tool_ask）→ 用户按**同一条回答命令**作答 → 选项 id 回到发起方。
/// 覆盖三态：选"本轮无围栏跑一次"才执行 / 选"放弃"不执行 / 整队作废（停止 = 拒绝）不执行；
/// 外加没有可回答的前端那一路（fail-closed：不执行）。
#[test]
fn tool_layer_ask_rides_the_decision_channel() {
    use crate::capabilities::conductor::api::{ConductorHandle, Ops, Output};
    use crate::capabilities::session::api::SessionEvent;
    use crate::kernel::domain::fence::{OPT_FENCE_ABORT, OPT_FENCE_UNFENCED};
    use crate::tests::builders::AskingRunner;
    use crate::tests::doubles::{abs, core_with_runner, decl, gw, module_of};
    use crate::tests::prelude::*;
    use std::sync::Arc;

    /// 跑一次"这一席的围栏装不上，先问用户"的生成。
    /// `answer = Some(选项 id)` 就等卡进队后按它作答；`stop = true` 则等卡进队后按**停止**（整队作废）。
    /// 返回：事件、工具行回执、"问过几次"、"真跑了几次"、那张卡的标题与详情。
    fn ask_case(
        allow_cards: bool,
        answer: Option<&str>,
        stop: bool,
    ) -> (Vec<SessionEvent>, Vec<String>, usize, usize, String) {
        // 发起方的规则：**只有**"本轮无围栏跑一次"才真跑；别的答案（放弃 / 没有回答）都不跑。
        let runner = Arc::new(AskingRunner::new(OPT_FENCE_UNFENCED));
        let mut member = BTreeMap::new();
        member.insert(
            "a".to_string(),
            vec![
                "{\"type\":\"tool\",\"module\":\"a\",\"name\":\"grep\",\"args\":{}}".to_string(),
                "{\"type\":\"say\",\"text\":\"做完了\"}".to_string(),
            ],
        );
        let mut a = module_of("a");
        a.root = abs(&["mods", "a"]);
        a.manifest
            .tools
            .insert("grep".to_string(), decl("python tools/grep.py"));
        let handle = ConductorHandle::spawn(core_with_runner(
            vec![a],
            gw(member, vec!["[]".into()]),
            Arc::clone(&runner),
        ))
        .expect("起核心线程");
        // 有交互前端在服务：工具层的提问端口接进裁决队（没有它这一趟就问不了）。
        if allow_cards {
            handle.allow_tool_cards();
        }
        let ops = Ops::from_handle(&handle);
        let sid = ops
            .sessions
            .create_work(work("w", WorkMode::Single, &["a"]))
            .expect("建会话")
            .0
            .sid;
        let answerer = {
            let ops = ops.clone();
            let sid = sid.clone();
            let answer = answer.map(|s| s.to_string());
            std::thread::spawn(move || {
                // 等卡进队：按墙钟上限等；固定迭代数在并行负载下会提前耗尽（不是卡没来）。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    if let Ok(Some(q)) = ops.sessions.open_queue(&sid) {
                        assert_eq!(
                            q.card.envelope.role, "tools",
                            "谁在问：工具层（与工具级确认同一信封）"
                        );
                        let ids: Vec<String> =
                            q.card.options.iter().map(|o| o.id.clone()).collect();
                        assert_eq!(
                            ids,
                            vec![OPT_FENCE_UNFENCED.to_string(), OPT_FENCE_ABORT.to_string()],
                            "选项 id 是契约（发起方给，通道原样渲染）"
                        );
                        if stop {
                            // 停止 = 拒绝：整队作废，等待方解开、按拒绝收场。
                            let _ = ops.sessions.stop(&sid);
                            return true;
                        }
                        // 回答走**唯一那条命令**（卡号 + 选项 id），处置归发起方。
                        ops.sessions
                            .answer_card(&sid, &q.card.id, answer.as_deref().unwrap_or(""), "")
                            .expect("按选项作答");
                        return true;
                    }
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            })
        };
        ops.sessions
            .say(&sid, "干活", Output::Final)
            .expect("跑一回合");
        let reached = answerer.join().expect("作答线程");
        let (batches, _head, _oldest) = handle.events().snapshot(Some(&sid), 0);
        let events: Vec<SessionEvent> = batches.into_iter().flat_map(|l| l.events).collect();
        let outs: Vec<String> = crate::tests::builders::tool_views(&events)
            .into_iter()
            .map(|v| v.output)
            .collect();
        let asked = runner.asked.lock().expect("锁").len();
        let ran = runner.calls.lock().expect("锁").len();
        let card = events
            .iter()
            .find_map(|e| match e {
                SessionEvent::DecisionCard { card, gate, .. } if gate == "tool_ask" => {
                    Some(format!("{}｜{}", card.message.title, card.message.detail))
                }
                _ => None,
            })
            .unwrap_or_default();
        assert_eq!(
            reached, allow_cards,
            "有前端就要等到卡（并作答/停止）；没有前端就该没有卡"
        );
        // 卡与回答都落盘（会话的事实）：重启后按转录重建时，已答过的不重问。
        if allow_cards && !stop {
            let (_, saved) = ops.history.open(&sid).expect("读转录");
            let kinds: Vec<String> = saved
                .iter()
                .filter_map(|ev| {
                    ev.get("type")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            assert!(
                kinds.iter().any(|k| k == "decision_card")
                    && kinds.iter().any(|k| k == "decision_answer"),
                "工具层的卡与它的回答都要落盘：{:?}",
                kinds
            );
        }
        (events, outs, asked, ran, card)
    }

    // ① 选"本轮无围栏跑一次"：工具真的执行了一次，回执如实标为无围栏。
    let (events, outs, asked, ran, card) = ask_case(true, Some(OPT_FENCE_UNFENCED), false);
    assert_eq!(asked, 1, "要问一次：{:?}", outs);
    assert_eq!(ran, 1, "选了跑一次才执行：{:?}", outs);
    assert!(card.contains("围栏的这一环装不上"), "{}", card);
    assert!(card.contains("错误码 5"), "详情要写清缺什么前提：{}", card);
    assert!(
        outs.iter().any(|o| o.contains("无围栏跑了一次")),
        "回执要如实说这次是无围栏跑的：{:?}",
        outs
    );
    let cards = events
        .iter()
        .filter(|e| matches!(e, SessionEvent::DecisionCard { gate, .. } if gate == "tool_ask"))
        .count();
    assert_eq!(cards, 1, "一次提问推一张卡（同一条队，不是第二条通道）");

    // ② 选"放弃这次调用"：不执行，回执如实说没执行。
    let (_ev, outs, asked, ran, _card) = ask_case(true, Some(OPT_FENCE_ABORT), false);
    assert_eq!(asked, 1, "{:?}", outs);
    assert_eq!(ran, 0, "放弃之后不许执行：{:?}", outs);
    assert!(
        outs.iter().any(|o| o.contains("没有执行")),
        "回执要如实：{:?}",
        outs
    );

    // ③ 用户按停止：整队作废 = 拒绝（不是放行），工具不执行。
    let (_ev, outs, asked, ran, _card) = ask_case(true, None, true);
    assert_eq!(asked, 1, "{:?}", outs);
    assert_eq!(ran, 0, "停止 = 拒绝，不许执行：{:?}", outs);

    // ④ 没有可回答的前端（纯终端 / e2e）：没有可点的选项 = 不执行（fail-closed）。
    let (events, outs, asked, ran, card) = ask_case(false, None, false);
    assert_eq!(asked, 0, "没有前端就没人能答，不发起裁决");
    assert_eq!(ran, 0, "不执行：{:?}", outs);
    assert!(card.is_empty(), "没有前端不该推卡：{}", card);
    assert!(
        outs.iter().any(|o| o.contains("没有可回答的前端")),
        "回执要如实说清为什么没执行：{:?}",
        outs
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::DecisionCard { gate, .. } if gate == "tool_ask")),
        "没有可回答的前端时一张卡都不该进队"
    );
}

/// 裁决队：先来后到（只有队首可答）、答案校验（旧卡 / 卡上没有的选项都挡下）、整队作废解开等待方。
#[test]
fn decision_door_queues_only_the_head_and_voids_waiters() {
    use crate::capabilities::session::api::{
        AnswerSlot, Answered, DecisionDoor, Pending, SessionEvent, OPT_BEGIN, OPT_TOOL_ALLOW,
    };
    use std::sync::Arc;

    let tool = || Pending::ToolApproval {
        agent: "甲".to_string(),
        module: Some("m0".to_string()),
        tool: "grep".to_string(),
        args: "{}".to_string(),
    };
    let door = Arc::new(DecisionDoor::default());
    assert!(door.is_empty(), "没进队 = 没在等");

    // 工具级确认（等在工作线程上）：进队 → 等 → 回答唤醒。
    let slot = AnswerSlot::new();
    let (id, evs) = door.push(None, tool(), "", Some(Arc::clone(&slot)));
    assert_eq!(id, "d1");
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::DecisionCard { .. })),
        "进队要推一张卡"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::Working { .. })),
        "等在工作线程上的那一关不推空闲（生成还在跑）"
    );
    let h = std::thread::spawn(move || slot.wait());
    // 旧卡号 / 卡上没有的选项都答不动它（校验属于当时那张卡），而且不留痕。
    assert!(door.answer("d404", OPT_TOOL_ALLOW, "").is_err(), "旧卡不行");
    assert!(door.answer(&id, OPT_BEGIN, "").is_err(), "别关的选项不行");
    assert!(!door.is_empty(), "被拒的回答不出队");
    match door.answer(&id, OPT_TOOL_ALLOW, "").expect("答队首那张") {
        Answered::Waiting(evs) => assert!(
            evs.iter().any(
                |e| matches!(e, SessionEvent::DecisionAnswer(a) if a.option == OPT_TOOL_ALLOW)
            ),
            "回答要落一条记录"
        ),
        Answered::Gate(..) => panic!("工具卡不该走核心关卡那条路"),
    }
    assert_eq!(
        h.join().expect("线程"),
        crate::capabilities::session::api::SlotWake::Answer(OPT_TOOL_ALLOW.to_string()),
        "回答要唤醒等待方"
    );
    assert!(door.is_empty(), "答完不再挂着");

    // 队列：先来后到——排在后面的那几张还不能答（它的答案会被"卡号不是队首"挡下）。
    door.push(None, Pending::ConfirmBegin, "", None);
    let slot2 = AnswerSlot::new();
    let (second, _) = door.push(None, tool(), "", Some(Arc::clone(&slot2)));
    assert!(
        door.answer(&second, OPT_TOOL_ALLOW, "").is_err(),
        "只有队首可答"
    );
    let h2 = std::thread::spawn(move || slot2.wait());
    // 整队作废（停止 / 关闭 = 拒绝）：排队的卡一律作废，等待方解开、按拒绝收场。
    assert_eq!(door.void(), vec!["d2".to_string(), "d3".to_string()]);
    assert_eq!(
        h2.join().expect("线程"),
        crate::capabilities::session::api::SlotWake::Stopped,
        "整队作废 = 用户停止（发起方按拒绝处置，不套用默认项）"
    );
    assert!(door.is_empty(), "作废之后不再挂着");
    assert!(door.queue().is_none(), "作废之后快照里没有卡");
}

/// 工具级确认**并入裁决通道**（端到端一条）：`ask` 命中 → 卡片进同一条队 → 按选项作答 → 工具才跑。
/// 三态都走一遍（放行 / 拒绝 / 本轮不再问），并核对卡与回答**落盘**（重启后不重问的那份事实）。
#[test]
fn tool_confirmation_rides_the_decision_channel() {
    use crate::capabilities::conductor::api::{ConductorHandle, Ops, Output};
    use crate::capabilities::session::api::{
        SessionEvent, OPT_TOOL_ALLOW, OPT_TOOL_ALLOW_TURN, OPT_TOOL_DENY,
    };
    use crate::tests::builders::{tool_line_texts, RecordingRunner};
    use crate::tests::doubles::{abs, core_with_runner, decl, gw, module_of};
    use crate::tests::prelude::*;
    use std::sync::{Arc, Mutex};

    /// 跑一次"这一席要问用户"的生成：等第一张工具卡进队、按 `answer` 作答（走**回答命令**那条路），
    /// 再等它跑完。返回：事件台上的事件、工具行、卡片张数、这一席的工具进程实际跑了几次。
    fn ask_case(
        answer: &str,
        calls: usize,
    ) -> (Vec<SessionEvent>, Vec<String>, Vec<String>, usize, usize) {
        let runner = Arc::new(RecordingRunner {
            calls: Mutex::new(Vec::new()),
            out: "ok".into(),
            ok: true,
        });
        let mut script: Vec<String> = Vec::new();
        for _ in 0..calls {
            script.push(
                "{\"type\":\"tool\",\"module\":\"a\",\"name\":\"grep\",\"args\":{}}".to_string(),
            );
        }
        script.push("{\"type\":\"say\",\"text\":\"做完了\"}".to_string());
        let mut member = BTreeMap::new();
        member.insert("a".to_string(), script);
        let mut a = module_of("a");
        a.root = abs(&["mods", "a"]);
        a.manifest
            .tools
            .insert("grep".to_string(), decl("python tools/grep.py"));
        let handle = ConductorHandle::spawn(core_with_runner(
            vec![a],
            gw(member, vec!["[]".into()]),
            Arc::clone(&runner),
        ))
        .expect("起核心线程");
        // 有交互前端在服务：工具级确认接进裁决队（纯终端同步生成不接，避免空等）。
        handle.allow_tool_cards();
        let ops = Ops::from_handle(&handle);
        let sid = ops
            .sessions
            .create_work(work("w", WorkMode::Single, &["a"]))
            .expect("建会话")
            .0
            .sid;
        // 粒度与问谁走**同一条设置路**（逐 agent 覆盖写回 meta，与界面里改配置同一条命令）。
        ops.sessions
            .edit(
                &sid,
                SessionEdit {
                    agents: vec![ConfigAgent {
                        name: "a".to_string(),
                        modules: vec!["a".to_string()],
                        model: String::new(),
                        chara: String::new(),
                    }],
                    tier: "host".to_string(),
                    guest: None,
                    pins: BTreeMap::new(),
                    permission: Some(PermissionLayer {
                        tools: BTreeMap::from([("a.grep".to_string(), Decision::Ask)]),
                        ..Default::default()
                    }),
                },
            )
            .expect("把这一席的粒度设成 ask");
        let answerer = {
            let ops = ops.clone();
            let sid = sid.clone();
            let answer = answer.to_string();
            std::thread::spawn(move || {
                // 等卡进队：按墙钟上限等；固定迭代数在并行负载下会提前耗尽（不是卡没来）。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    if let Ok(Some(q)) = ops.sessions.open_queue(&sid) {
                        assert_eq!(
                            q.card
                                .options
                                .iter()
                                .map(|o| o.id.clone())
                                .collect::<Vec<_>>(),
                            vec![
                                OPT_TOOL_ALLOW.to_string(),
                                OPT_TOOL_DENY.to_string(),
                                OPT_TOOL_ALLOW_TURN.to_string()
                            ],
                            "工具卡的选项 id 是契约：allow / deny / full"
                        );
                        assert_eq!(q.card.envelope.role, "tools", "谁在问：工具层");
                        // 回答走**唯一那条命令**（卡号 + 选项 id）。
                        ops.sessions
                            .answer_card(&sid, &q.card.id, &answer, "")
                            .expect("按选项作答");
                        return true;
                    }
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            })
        };
        ops.sessions
            .say(&sid, "干活", Output::Final)
            .expect("跑一回合");
        assert!(answerer.join().expect("作答线程"), "工具卡要进队并等回答");
        assert!(
            ops.sessions.open_queue(&sid).expect("取卡").is_none(),
            "答完不再挂着"
        );
        let (batches, _head, _oldest) = handle.events().snapshot(Some(&sid), 0);
        let events: Vec<SessionEvent> = batches.into_iter().flat_map(|l| l.events).collect();
        let cards = events
            .iter()
            .filter(
                |e| matches!(e, SessionEvent::DecisionCard { gate, .. } if gate == "tool_approval"),
            )
            .count();
        let lines = tool_line_texts(&events);
        let outs: Vec<String> = crate::tests::builders::tool_views(&events)
            .into_iter()
            .map(|v| v.output)
            .collect();
        let ran = runner.calls.lock().expect("锁").len();
        // 卡与回答都落盘（会话的事实）：重启后不重问已答的。
        let (_, saved) = ops.history.open(&sid).expect("读转录");
        let kinds: Vec<String> = saved
            .iter()
            .filter_map(|ev| {
                ev.get("type")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        assert!(
            kinds.iter().any(|k| k == "decision_card")
                && kinds.iter().any(|k| k == "decision_answer"),
            "工具卡与它的回答都要落盘：{:?}",
            kinds
        );
        assert_eq!(outs.len(), calls.max(1), "每次调用都要留一条工具结果");
        (events, lines, outs, cards, ran)
    }

    // ① 放行：卡进队 → 答 allow → 工具真的执行。
    let (_ev, lines, outs, cards, ran) = ask_case(OPT_TOOL_ALLOW, 1);
    assert_eq!(cards, 1, "先问一次：{:?}", lines);
    assert_eq!(ran, 1, "放行之后工具执行一次：{:?}", lines);
    assert_eq!(outs, vec!["ok".to_string()], "{:?}", lines);

    // ② 拒绝：不执行，回一条"用户没有放行"的结果给模型（如实说，且不启动进程）。
    let (_ev, lines, outs, cards, ran) = ask_case(OPT_TOOL_DENY, 1);
    assert_eq!(cards, 1, "{:?}", lines);
    assert_eq!(ran, 0, "拒绝之后不启动工具进程：{:?}", lines);
    assert!(
        outs.iter().any(|o| o.contains("没有放行")),
        "拒绝的理由要如实回给模型：{:?}",
        outs
    );

    // ③ 本轮不再问：一次回复里两次调用，答 full → 只问一次、两次都执行（且**不落盘**成策略）。
    let (_ev, lines, outs, cards, ran) = ask_case(OPT_TOOL_ALLOW_TURN, 2);
    assert_eq!(cards, 1, "full 之后本轮不再问：{:?}", lines);
    assert_eq!(ran, 2, "两次调用都要执行：{:?}", lines);
    assert!(outs.iter().all(|o| o == "ok"), "{:?}", outs);
}
