//! 权限能力（permission）的用例：白/黑名单语义、覆盖解析、路径匹配、落点与提交/拉取作用域。
//!
//! 归属：这些用例钉的是**权限规则本身**（T1），以及内置工具/工作区提交两个执行点对它的消费（T1/T2）。
//! 平台围栏的真机验收在 tests/<平台>/（T4）。

use crate::capabilities::permission::api::{
    validate, Granularity, Permissions, PermissionsOverride,
};
use crate::capabilities::workspace::api::Place;
use crate::tests::doubles::{abs, run_builtin, test_sandbox, InMemorySysIo};

fn paths(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// 未配置 = 默认：整棵工作区可读、可提交；模块目录只读（无写授权）。
#[test]
fn defaults_allow_the_whole_workspace_and_keep_modules_read_only() {
    let p = Permissions::default();
    assert!(p.read_ok("a/b.txt"));
    assert!(p.write_ok("任何/路径"));
    assert!(!p.module_write_ok("m"));
}

/// 白名单**取代**默认；黑名单**只做减法**；两者重叠时黑大于白。
#[test]
fn whitelist_replaces_default_while_blacklist_only_subtracts() {
    // 只有黑名单：默认仍在，减去被禁的那块。
    let deny_only = Permissions {
        deny: paths(&["secret"]),
        ..Default::default()
    };
    assert!(deny_only.read_ok("docs/a"));
    assert!(!deny_only.read_ok("secret/a"));
    assert!(deny_only.write_ok("docs/a"));
    assert!(!deny_only.write_ok("secret/a"));

    // 只有读白名单：读的默认失效，写的默认仍在（读写各自独立）。
    let allow_read = Permissions {
        allow_read: paths(&["docs"]),
        ..Default::default()
    };
    assert!(allow_read.read_ok("docs/a"));
    assert!(!allow_read.read_ok("src/a"));
    assert!(allow_read.write_ok("src/a"));

    // 白 + 黑重叠：黑赢。
    let both = Permissions {
        allow_write: paths(&["web"]),
        deny: paths(&["web/secrets"]),
        ..Default::default()
    };
    assert!(both.write_ok("web/ok.txt"));
    assert!(!both.write_ok("web/secrets/key"));
    assert!(!both.write_ok("api/x"));
}

/// 匹配按**路径组件**做：`web` 不匹配 `website`；`.` 代表整棵。
#[test]
fn matching_is_component_aware() {
    let p = Permissions {
        allow_write: paths(&["web"]),
        ..Default::default()
    };
    assert!(p.write_ok("web"));
    assert!(p.write_ok("web/a/b"));
    assert!(!p.write_ok("website/a"));
    assert!(!p.write_ok("web-other"));
    let all = Permissions {
        allow_read: paths(&["."]),
        ..Default::default()
    };
    assert!(all.read_ok("any/where"));
}

/// 覆盖只替换**显式给出**的字段；显式空集合 = 明确的"没有白名单"（回到默认），与"没给"分得开。
#[test]
fn override_replaces_only_the_fields_it_gives() {
    let base = Permissions {
        allow_read: paths(&["docs"]),
        deny: paths(&["docs/secret"]),
        ..Default::default()
    };
    let over = PermissionsOverride {
        allow_write: Some(paths(&["web"])),
        ..Default::default()
    };
    let out = base.apply(&over);
    assert_eq!(out.allow_read, paths(&["docs"]));
    assert_eq!(out.deny, paths(&["docs/secret"]));
    assert_eq!(out.allow_write, paths(&["web"]));

    let reset = PermissionsOverride {
        allow_read: Some(Vec::new()),
        ..Default::default()
    };
    assert!(base.apply(&reset).read_ok("anything"));
}

/// 条目必须是干净的**相对**路径：绝对路径、`..`、空段、盘符一律拒收。
#[test]
fn entries_must_be_clean_relative_paths() {
    let ok = |e: &str| {
        validate(&Permissions {
            allow_read: paths(&[e]),
            ..Default::default()
        })
        .is_ok()
    };
    assert!(ok("docs/a"));
    assert!(ok("."));
    for bad in ["/abs/x", "..", "a/../b", "a//b", r"C:\win", ""] {
        assert!(!ok(bad), "应当拒绝：{:?}", bad);
    }
    let p = Permissions {
        allow_write: paths(&["../escape"]),
        ..Default::default()
    };
    assert!(validate(&p).is_err());
}

/// 模块写授权按模块 id 判定。
#[test]
fn module_write_authorization_is_per_module() {
    let p = Permissions {
        module_write: paths(&["codegen"]),
        ..Default::default()
    };
    assert!(p.module_write_ok("codegen"));
    assert!(!p.module_write_ok("render"));
}

/// 落点规则：共享区读写受白黑名单；私有沙箱永远全权；模块目录默认只读、`userdata/` 例外。
#[test]
fn sandbox_places_follow_the_permission_set() {
    let mut sb = test_sandbox("a1", &["data"]);
    let shared = sb.shared.clone();
    let private = sb.private.clone();
    let module = sb.modules.get("data").expect("模块目录").clone();
    sb.permissions = Permissions {
        allow_read: paths(&["docs"]),
        allow_write: paths(&["docs"]),
        ..Default::default()
    };
    // 共享区：白名单外读写都拒；白名单内放行。
    assert!(!sb.can_read(&Place::Shared, &shared.join("src").join("x")));
    assert!(sb.can_read(&Place::Shared, &shared.join("docs").join("x")));
    assert!(!sb.can_write(&Place::Shared, &shared.join("src").join("x")));
    assert!(sb.can_write(&Place::Shared, &shared.join("docs").join("x")));
    // 私有沙箱：不受白名单影响，永远全权。
    assert!(sb.can_read(&Place::Private, &private.join("src").join("x")));
    assert!(sb.can_write(&Place::Private, &private.join("src").join("x")));
    // 模块目录：默认只读；userdata 例外。
    let module_place = Place::Module("data".to_string());
    assert!(!sb.can_write(&module_place, &module.join("script.py")));
    assert!(sb.can_write(&module_place, &module.join("userdata").join("state.json")));
}

/// 内置工具：读/写越出白名单都如实拒绝；私有沙箱照常。
#[test]
fn builtin_tools_respect_the_scope() {
    let io = InMemorySysIo::new();
    let mut sb = test_sandbox("a1", &[]);
    sb.permissions.allow_write = paths(&["docs"]);
    sb.permissions.allow_read = paths(&["docs"]);

    let out_of_scope = abs(&["demo", "work", "src", "x.txt"]);
    let bad = run_builtin(
        &sb,
        &io,
        "write",
        &serde_json::json!({"path": out_of_scope.to_string_lossy(), "content": "x"}).to_string(),
    );
    assert!(!bad.ok, "写白名单外必须拒绝：{}", bad.output);

    let in_scope = abs(&["demo", "work", "docs", "x.txt"]);
    let ok = run_builtin(
        &sb,
        &io,
        "write",
        &serde_json::json!({"path": in_scope.to_string_lossy(), "content": "x"}).to_string(),
    );
    assert!(ok.ok, "写白名单内必须放行：{}", ok.output);

    let denied = run_builtin(
        &sb,
        &io,
        "read",
        &serde_json::json!({"path": abs(&["demo", "work", "src", "y.txt"]).to_string_lossy()})
            .to_string(),
    );
    assert!(!denied.ok, "读白名单外必须拒绝：{}", denied.output);

    let private_ok = run_builtin(
        &sb,
        &io,
        "write",
        &serde_json::json!({"path": abs(&["demo", "a1", "scratch.txt"]).to_string_lossy(), "content": "x"})
            .to_string(),
    );
    assert!(
        private_ok.ok,
        "私有沙箱不受白名单约束：{}",
        private_ok.output
    );
}

/// 围栏派生：模块目录默认只读（进 ro_tree）；`userdata/` 的派发看**注入的事实**，domain 不读盘。
/// `standalone` 的缺省工作目录同样按这条事实回退。
#[test]
fn fence_spec_scopes_module_dirs_and_userdata() {
    use crate::capabilities::tools::api::FenceSpec;
    let mut sb = test_sandbox("a1", &[]);
    let module = abs(&["mods", "data"]);
    sb.modules.insert("data".to_string(), module.clone());

    // 事实：没有 userdata → 不进 rw，模块根仍进 ro_tree。
    let spec = FenceSpec::from_sandbox(&sb, false);
    assert!(spec.ro_tree.contains(&module), "模块目录默认只读");
    assert!(
        !spec.rw.contains(&module.join("userdata")),
        "事实说没有 userdata 就不派这条落点"
    );
    assert!(!spec.rw.contains(&module), "未授权时整块模块目录不进 rw");
    assert_eq!(spec.private, sb.private, "HOME/TEMP 落在私有沙箱");

    // 事实：有 userdata → 进 rw。
    sb.modules_with_userdata.insert("data".to_string());
    let spec = FenceSpec::from_sandbox(&sb, false);
    assert!(
        spec.rw.contains(&module.join("userdata")),
        "事实说有 userdata 就可写"
    );

    // 授权后：模块整块进 rw，不再进 ro_tree。
    sb.permissions.module_write = vec!["data".to_string()];
    let spec = FenceSpec::from_sandbox(&sb, true);
    assert!(spec.rw.contains(&module));
    assert!(!spec.ro_tree.contains(&module));
    assert!(spec.net, "net 随参数透传");
}

/// `standalone` 的缺省工作目录按**注入的事实**回退：没有 userdata 时退回模块根，不派不存在的落点。
#[test]
fn standalone_falls_back_when_userdata_is_absent() {
    use crate::capabilities::tools::api::FenceSpec;
    let module = abs(&["mods", "m0"]);

    let spec = FenceSpec::standalone(&module, None, false);
    assert!(
        !spec.rw.contains(&module.join("userdata")),
        "事实说没有就不派"
    );
    assert_eq!(
        spec.private_or_cwd(),
        module,
        "没有 userdata 时工作目录退回模块根"
    );
    assert_eq!(spec.cwd, module, "cwd 仍是模块根");

    let spec = FenceSpec::standalone(&module, None, true);
    assert!(spec.rw.contains(&module.join("userdata")), "事实说有就派");
    assert_eq!(
        spec.private,
        module.join("userdata"),
        "缺省工作目录 = userdata"
    );

    // 显式 work_root：事实说有 userdata 时补进 rw，说没有时不补。
    let work = abs(&["mods", "work"]);
    let with = FenceSpec::standalone(&module, Some(&work), true);
    assert!(with.rw.contains(&work) && with.rw.contains(&module.join("userdata")));
    let without = FenceSpec::standalone(&module, Some(&work), false);
    assert!(without.rw.contains(&work) && !without.rw.contains(&module.join("userdata")));
}

/// 提交与拉取的作用域判定（工作区工具消费的那一份纯函数）：越界整条拒绝，不带半次提交。
#[test]
fn work_commit_and_pull_are_scoped() {
    use crate::capabilities::conductor::service::work_tools::{check_readable, check_writable};
    let p = Permissions {
        allow_read: paths(&["docs"]),
        allow_write: paths(&["docs"]),
        deny: paths(&["docs/secret"]),
        ..Default::default()
    };
    assert!(check_readable(&p, &paths(&["docs/a"])).is_ok());
    assert!(check_readable(&p, &paths(&["src/a"])).is_err());
    assert!(check_writable(&p, &paths(&["docs/a"]), &[]).is_ok());
    assert!(check_writable(&p, &paths(&["docs/secret/key"]), &[]).is_err());
    // 删除也在作用域之内（不能靠 deletes 绕过白名单）。
    assert!(check_writable(&p, &[], &paths(&["src/a"])).is_err());
    // 任一条越界 → 整次拒绝。
    assert!(check_writable(&p, &paths(&["docs/a", "src/b"]), &[]).is_err());
}

/// `full` 不确认任何工具；`ask` 只确认表里命中的（空表 = 不确认）。
#[test]
fn ask_list_only_applies_under_ask_granularity() {
    let full = Permissions {
        granularity: Granularity::Full,
        ask: paths(&["write"]),
        ..Default::default()
    };
    assert!(!full.should_ask("write"), "full 一律不确认");

    let ask = Permissions {
        granularity: Granularity::Ask,
        ask: paths(&["write", "m.t"]),
        ..Default::default()
    };
    assert!(ask.should_ask("write"));
    assert!(ask.should_ask("m.t"));
    assert!(!ask.should_ask("read"));

    assert!(
        !Permissions::default().should_ask("write"),
        "空 ask 表不确认"
    );
}

/// 工具级确认：`ask` 命中时先问用户——拒绝不执行、放行才跑、`full` 本轮不再问。
#[test]
fn ask_tools_are_confirmed_before_execution() {
    use crate::capabilities::collab::service::tool_loop::{run_batch, ConfirmGate, ToolConfirm};
    use crate::capabilities::session::api::{
        SessionEvent, OPT_TOOL_ALLOW, OPT_TOOL_DENY, OPT_TOOL_FULL,
    };
    use crate::tests::builders::{member_with_tools, RecordingRunner};
    use std::sync::Arc;

    let runner = Arc::new(RecordingRunner::new("工具输出", true));
    let mut m = member_with_tools("a1", vec![], Arc::clone(&runner));
    let mut tools = m.tools.take().expect("成员工具环境");
    tools.sandbox.permissions = Permissions {
        granularity: Granularity::Ask,
        ask: paths(&["m0.grep"]),
        ..Default::default()
    };
    let plan = vec![(Some("m0".to_string()), "grep".to_string(), "{}".to_string())];
    let mut sink = |_e: SessionEvent| {};

    // ① 拒绝：问过用户、工具没执行、回一条"用户拒绝"。
    let mut asked = Vec::new();
    let mut deny = |req: &ToolConfirm, _sink: &mut dyn FnMut(SessionEvent)| -> String {
        asked.push(req.tool.clone());
        OPT_TOOL_DENY.to_string()
    };
    let mut gate = ConfirmGate {
        full: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        confirm: &mut deny,
    };
    let done = run_batch(&mut tools, &plan, Some(&mut gate), &mut sink);
    assert_eq!(asked, vec!["grep".to_string()], "ask 命中要问用户");
    assert!(!done[0].1.ok);
    assert!(
        done[0].1.output.contains("没有放行"),
        "拒绝理由要如实：{}",
        done[0].1.output
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "拒绝后不得启动工具进程"
    );

    // ② 放行：工具真的执行了一次。
    let mut allow = |_req: &ToolConfirm, _sink: &mut dyn FnMut(SessionEvent)| -> String {
        OPT_TOOL_ALLOW.to_string()
    };
    let mut gate = ConfirmGate {
        full: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        confirm: &mut allow,
    };
    let done = run_batch(&mut tools, &plan, Some(&mut gate), &mut sink);
    assert!(done[0].1.ok, "{}", done[0].1.output);
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        1,
        "放行后要执行一次"
    );

    // ③ full：第一次答"本轮不再问"，后面两次调用直接放行、不再问。
    let mut asked_times = 0usize;
    let mut full_answer = |_req: &ToolConfirm, _sink: &mut dyn FnMut(SessionEvent)| -> String {
        asked_times += 1;
        OPT_TOOL_FULL.to_string()
    };
    let mut gate = ConfirmGate {
        full: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        confirm: &mut full_answer,
    };
    let three = vec![
        (Some("m0".to_string()), "grep".to_string(), "{}".to_string()),
        (Some("m0".to_string()), "grep".to_string(), "{}".to_string()),
        (Some("m0".to_string()), "grep".to_string(), "{}".to_string()),
    ];
    let done = run_batch(&mut tools, &three, Some(&mut gate), &mut sink);
    assert!(done.iter().all(|(_, o)| o.ok), "full 之后都要执行");
    assert_eq!(asked_times, 1, "full 之后本轮不再问");
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
        Some(OPT_TOOL_ALLOW.to_string()),
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
    assert_eq!(h2.join().expect("线程"), None, "作废 = 拒绝（不执行）");
    assert!(door.is_empty(), "作废之后不再挂着");
    assert!(door.queue().is_none(), "作废之后快照里没有卡");
}

/// 工具级确认**并入裁决通道**（端到端一条）：`ask` 命中 → 卡片进同一条队 → 按选项作答 → 工具才跑。
/// 三态都走一遍（放行 / 拒绝 / 本轮不再问），并核对卡与回答**落盘**（重启后不重问的那份事实）。
#[test]
fn tool_confirmation_rides_the_decision_channel() {
    use crate::capabilities::conductor::api::{ConductorHandle, Ops, Output};
    use crate::capabilities::session::api::{
        SessionEvent, OPT_TOOL_ALLOW, OPT_TOOL_DENY, OPT_TOOL_FULL,
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
                        permissions: Some(PermissionsOverride {
                            granularity: Some(Granularity::Ask),
                            ask: Some(paths(&["a.grep"])),
                            ..Default::default()
                        }),
                    }],
                    tier: "host".to_string(),
                    base: None,
                    pins: BTreeMap::new(),
                    net: false,
                },
            )
            .expect("把这一席的粒度设成 ask");
        let answerer = {
            let ops = ops.clone();
            let sid = sid.clone();
            let answer = answer.to_string();
            std::thread::spawn(move || {
                for _ in 0..2000 {
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
                                OPT_TOOL_FULL.to_string()
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
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                false
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
    let (_ev, lines, outs, cards, ran) = ask_case(OPT_TOOL_FULL, 2);
    assert_eq!(cards, 1, "full 之后本轮不再问：{:?}", lines);
    assert_eq!(ran, 2, "两次调用都要执行：{:?}", lines);
    assert!(outs.iter().all(|o| o == "ok"), "{:?}", outs);
}
