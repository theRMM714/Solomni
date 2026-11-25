//! 目的：会话视图与动作——会话创建、设置、权限档、思考档、工作名与配置快照。
//! 管：动作入参校验、视图字段、会话编辑在生成中的拒绝、档位切换的落盘。
//! 不管：压缩（`compaction.rs`）、动作目录本身（`tools.rs`）。
//! 联动：`src/units/runtime/api/action.rs` 与 `src/units/session/api`。

use super::super::doubles::{decl, model_draft, module_of, module_with_runtimes};
use super::super::{ops_with, single_work, slow_ops};
use super::support::spawn;
use crate::units::catalog::registry::api::AgentMeta;
use crate::units::runtime::api::{Acted, ActionCall, Caller};
use crate::units::work::api::{ConfigAgent, SessionEdit, WorkSpec};
use crate::presentation::faces::Faces;

use crate::units::conversation::session::api::{CUSTOM, SINGLE};
use crate::units::runtime::api::Output;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn delete_past_an_archive_reports_the_casualties_and_needs_a_confirm() {
    use crate::units::lifecycle::api::RewindTarget;
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "第一句".to_string(),
            "第二句".to_string(),
            "第三句".to_string(),
        ],
    );
    let handle =
        crate::units::runtime::api::Handle::spawn(super::super::doubles::core_with_gateway(
            vec![module_of("a")],
            super::super::doubles::ScriptGateway::new(member, vec![]),
        ))
        .expect("起核心线程");
    let faces = crate::presentation::faces::Faces::from_handle(&handle);
    let mut st = faces.registry.settings().expect("读设置");
    st.compact_at_percent = 0;
    st.sandbox.policy.windows.write_acl = true;
    faces.registry.set_settings(st).expect("写设置");
    let sid = faces
        .work
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    faces
        .turn
        .say(
            &sid,
            "先说一句",
            crate::units::runtime::api::Output::Final,
        )
        .expect("第一轮");
    faces
        .runtime
        .rewind(&sid, RewindTarget::Archive(1))
        .expect("留档一次");
    faces
        .turn
        .say(
            &sid,
            "留档之后再说",
            crate::units::runtime::api::Output::Final,
        )
        .expect("第二轮");

    let err = faces
        .runtime
        .rewind(&sid, RewindTarget::Delete(1))
        .expect_err("删除点之后的留档要先确认");
    assert!(err.contains("作废后不可恢复"), "{err}");
    assert!(err.contains("1 个留档"), "{err}");

    handle
        .rewind_with_reset(&sid, RewindTarget::Delete(1), true)
        .expect("确认后删除");
    faces
        .turn
        .say(
            &sid,
            "删完继续",
            crate::units::runtime::api::Output::Final,
        )
        .expect("删除后还能继续");
}

#[test]
fn provider_usage_is_exposed_in_session_view_and_persisted() {
    use crate::units::llm::api::TokenUsage;
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"完成\"}".to_string()],
    );
    let gateway = super::super::doubles::ScriptGateway::new(member, vec![])
        .with_usage(TokenUsage::new(900, 120, Some(1020)));
    let handle = crate::units::runtime::api::Handle::spawn(
        super::super::doubles::core_with_gateway(vec![module_of("a")], gateway),
    )
    .expect("起核心线程");
    let faces = crate::presentation::faces::Faces::from_handle(&handle);
    let sid = faces
        .work
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    faces
        .turn
        .say(
            &sid,
            "做一下",
            crate::units::runtime::api::Output::Final,
        )
        .expect("一轮");

    let views = faces
        .work
        .session_views(&faces.history.list().expect("列表"))
        .expect("视图");
    let view = views.iter().find(|v| v.sid == sid).expect("会话在视图里");
    let ctx = view.context.expect("单 agent 会话有上下文用量");
    assert_eq!(ctx.total_tokens, 1020);
    assert_eq!(ctx.prompt_tokens, 900);
    assert!(ctx.exact, "供应商回的 usage 要标成精确");
    assert!(ctx.window > 0, "窗口来自登记处（缺省 128K）");
    let totals = view.usage_totals;
    assert_eq!(totals.calls, 1, "累计花费要记住调用次数");
    assert_eq!(totals.prompt, 900, "累计要记住输入总量");
    assert_eq!(totals.completion, 120, "累计要记住输出总量");
    assert_eq!(totals.cache_read, 0, "这份 usage 没报缓存读");

    // 落盘：刷新/重建读的是同一条 usage 事件，不靠内存里那份。
    let (_, events) = faces.history.open(&sid).expect("读流水");
    assert!(
        events.iter().any(|e| {
            e.get("type").and_then(|t| t.as_str()) == Some("usage")
                && e.get("total_tokens").and_then(|n| n.as_u64()) == Some(1020)
        }),
        "流水里要有 usage 事件"
    );
    let last =
        crate::units::conversation::session::api::last_usage(&events).expect("读回用量");
    assert_eq!(last.total_tokens, 1020);

    // 跨回合累加：第二轮的用量叠在 meta 的同一本账上，不覆盖也不清零。
    faces
        .turn
        .say(
            &sid,
            "再做一件",
            crate::units::runtime::api::Output::Final,
        )
        .expect("第二轮");
    let views = faces
        .work
        .session_views(&faces.history.list().expect("列表"))
        .expect("视图");
    let totals = views
        .iter()
        .find(|v| v.sid == sid)
        .expect("会话在视图里")
        .usage_totals;
    assert_eq!(totals.calls, 2, "第二轮也要累计调用次数");
    assert_eq!(totals.prompt, 1800, "输入按轮相加");
    assert_eq!(totals.completion, 240, "输出按轮相加");
}

#[test]
fn restore_work_commit_is_non_destructive_and_records_a_notice() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    let sid = faces
        .work
        .create_work(single_work("w-restore", &["a"]))
        .expect("建会话")
        .0
        .sid;
    assert!(
        faces
            .work
            .upload(&sid, "a.txt", b"one", false)
            .expect("投喂一")
    );
    assert!(
        faces
            .work
            .upload(&sid, "a.txt", b"two", true)
            .expect("投喂二")
    );
    let log = faces.work.work_log(&sid, 10, None).expect("提交历史");
    let head = log.head.as_ref().expect("有 head").id;
    let older = log
        .commits
        .iter()
        .map(|c| c.id)
        .find(|id| *id != head)
        .expect("有更早的提交");

    // 陈旧 expect_head：拒绝并说清当前 head。
    let err = faces
        .actions
        .act(ActionCall {
            id: "restore_work_commit".to_string(),
            args: serde_json::json!({ "session_id": sid, "commit": older, "expect_head": 0 }),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("已经变化"), "{}", err);

    // 正确 expect_head：回滚，目标之后的提交仍保留。
    let done = faces
        .actions
        .act(ActionCall {
            id: "restore_work_commit".to_string(),
            args: serde_json::json!({ "session_id": sid, "commit": older, "expect_head": head }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("回滚");
    assert!(matches!(done, Acted::Done(_)), "回滚是同步完成动作");
    let back = faces.work.work_log(&sid, 10, None).expect("回滚后的历史");
    assert_eq!(back.head.as_ref().map(|h| h.id), Some(older));
    assert_eq!(
        back.commits.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![head, older],
        "非破坏性：目标之后的提交仍在"
    );
    assert_eq!(
        back.commits.iter().map(|c| c.on_head).collect::<Vec<_>>(),
        vec![false, true],
        "旧 head 标成已离开当前线"
    );

    // 落一条 notice 行（有 id、可回放），内容点名 from/to 与"仍保留"。
    let (_, events) = faces.history.open(&sid).expect("读流水");
    let notice = events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flatten()
        .find(|l| {
            l.get("kind").and_then(|k| k.as_str()) == Some("notice")
                && l.get("line")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s.contains("工作区回滚"))
        })
        .expect("回滚要留可回放的 notice 行");
    let text = notice.get("line").and_then(|s| s.as_str()).unwrap_or("");
    assert!(text.contains(&format!("提交 #{}", older)), "{}", text);
    assert!(text.contains("仍保留"), "{}", text);
}

#[test]
fn unique_work_name_falls_back_and_appends_a_suffix() {
    use crate::units::conversation::session::api::unique_work_name;
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    let exists = |n: &str| faces.work.exists(n).expect("查占用");
    assert_eq!(unique_work_name("", "single", exists), "single");
    assert_eq!(unique_work_name("  取名  ", "single", exists), "取名");
    faces
        .work
        .create_work(single_work("w", &["a"]))
        .expect("建会话");
    assert_eq!(unique_work_name("w", "x", exists), "w-2");
    faces
        .work
        .create_work(single_work("w-2", &["a"]))
        .expect("建会话");
    assert_eq!(unique_work_name("w", "x", exists), "w-3");
}

#[test]
fn single_mode_merges_multiple_agents_into_one_transient() {
    let (_h, faces) = ops_with(vec![module_of("a"), module_of("b")], Vec::new());
    faces
        .registry
        .upsert_agent("甲", &["a".to_string(), "b".to_string()], "", "", "")
        .expect("建 agent");
    faces
        .registry
        .upsert_agent("乙", &["b".to_string()], "", "", "")
        .expect("建 agent");
    let picked: Vec<AgentMeta> = faces
        .registry
        .agents()
        .expect("读登记处")
        .iter()
        .map(crate::units::catalog::registry::api::agent_meta)
        .collect();
    let (opened, _) = faces
        .work
        .create_work(WorkSpec {
            flows: None,
            freshness: None,
            name: "组合".to_string(),
            kind: SINGLE.to_string(),
            agents: picked,
            task: None,
            delegate: false,
            permission: None,
            tier: crate::units::exec_model::api::Tier::Host,
            guest: None,
            workspace: None,
        })
        .expect("建工作");
    assert_eq!(
        opened.agents,
        vec!["组合".to_string()],
        "多个点名并成一个临时组合"
    );
    let (meta, _) = faces.history.open("组合").expect("读 meta");
    let mut modules = meta.agents[0].modules.clone();
    modules.sort();
    assert_eq!(
        modules,
        vec!["a".to_string(), "b".to_string()],
        "模块去重（顺序随登记处遍历序，不额外断言）"
    );
    assert!(meta.agents[0].transient, "并出来的「组合」是临时 agent");
}

/// 目的：临时拼装——省略 kind、只给流程 id 建会话，落盘的类型是 custom，流程 id 记在元数据里。
#[test]
fn an_inline_assembly_creates_a_custom_session_type() {
    let (_h, faces) = ops_with(vec![module_of("a"), module_of("b")], Vec::new());
    let mut spec = super::super::doubles::collab_work("拼的", &["a", "b"], false, "做点事");
    spec.kind = String::new();
    spec.flows = Some(vec!["discuss".to_string()]);
    let (opened, _) = faces.work.create_work(spec).expect("临时拼装建会话");
    assert_eq!(opened.agents.len(), 2, "名单照给的进会话");
    let (meta, _) = faces.history.open("拼的").expect("读 meta");
    assert_eq!(meta.spec.kind, CUSTOM, "临时拼装落成 custom 类型");
    assert_eq!(meta.spec.name, "临时组合", "类型名给界面渲染");
    assert_eq!(meta.spec.flow_ids(), vec!["discuss"]);
    assert!(meta.spec.driven(), "带流程 = 由流程驱动");
}
#[test]
fn editing_is_refused_while_a_session_is_generating() {
    let (_h, faces, _ticks) = slow_ops(vec![module_of("a")]);
    let sid = faces
        .work
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;
    // 编辑体照抄当前配置（空名单会被"至少要有一个 agent"挡掉，那是另一条规则）。
    let cfg = faces.work.config(&sid).expect("读配置");
    let edit = || SessionEdit {
        agents: cfg.agents.clone(),
        tier: "host".to_string(),
        guest: None,
        pins: std::collections::BTreeMap::new(),
        permission: None,
        shared: None,
    };
    faces.work.edit(&sid, edit()).expect("没在生成时可以改");

    let worker = {
        let turn = Arc::clone(&faces.turn);
        let sid = sid.clone();
        std::thread::spawn(move || turn.say(&sid, "慢慢来", Output::Stream))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !faces.turn.is_running(&sid) {
        assert!(Instant::now() < deadline, "生成没有启动");
        std::thread::sleep(Duration::from_millis(5));
    }
    let err = faces.work.edit(&sid, edit()).unwrap_err();
    assert!(err.contains("正在生成中"), "生成中必须拒绝改配置：{err}");

    assert!(!faces.runtime.stop(&sid).is_empty(), "停掉它");
    let _ = worker.join();
    faces.work.edit(&sid, edit()).expect("收尾后可以改");
    assert!(
        faces.work.edit("没这个会话", edit()).is_err(),
        "无此会话要如实报错"
    );
}

#[test]
fn set_settings_action_accepts_max_guests() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    // Windows 上 require 围栏必须授权写 ACL，否则设置本身不合法；这条只验动作参数。
    let mut st = faces.registry.settings().expect("读设置");
    st.sandbox.policy.windows.write_acl = true;
    faces
        .registry
        .set_settings(st)
        .expect("准备 Windows 合法的沙箱策略");
    faces
        .actions
        .act(ActionCall {
            id: "set_settings".to_string(),
            args: serde_json::json!({ "max_guests": 5 }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("set_settings 应接受 max_guests");
    assert_eq!(faces.registry.settings().expect("读设置").max_guests, 5);
}

#[test]
fn set_settings_action_accepts_storage() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    let mut st = faces.registry.settings().expect("读设置");
    st.sandbox.policy.windows.write_acl = true;
    faces
        .registry
        .set_settings(st)
        .expect("准备 Windows 合法的沙箱策略");
    faces
        .actions
        .act(ActionCall {
            id: "set_settings".to_string(),
            args: serde_json::json!({ "storage": {
                "periodic_gc": false,
                "gc_interval_minutes": 30,
                "cache_cap_gib": 7,
                "cache_keep_days": 11,
                "build_volume_gib": 9,
                "min_free_gib": 3,
                "log_keep_days": 5
            } }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("set_settings 应接受 storage");
    let got = faces.registry.settings().expect("读设置").storage;
    assert!(!got.periodic_gc);
    assert_eq!(got.gc_interval_minutes, 30);
    assert_eq!(got.cache_cap_gib, 7);
    assert_eq!(got.cache_keep_days, 11);
    assert_eq!(got.build_volume_gib, 9);
    assert_eq!(got.min_free_gib, 3);
    assert_eq!(got.log_keep_days, 5);
}

#[test]
fn set_settings_action_accepts_compact_options() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    let mut st = faces.registry.settings().expect("读设置");
    st.sandbox.policy.windows.write_acl = true;
    faces
        .registry
        .set_settings(st)
        .expect("准备 Windows 合法的沙箱策略");
    faces
        .actions
        .act(ActionCall {
            id: "set_settings".to_string(),
            args: serde_json::json!({ "compact_keep_percent": 20, "compact_model": "m" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("set_settings 应接受压缩选项");
    let got = faces.registry.settings().expect("读设置");
    assert_eq!(got.compact_keep_percent, 20);
    assert_eq!(got.compact_model, "m");

    let err = faces
        .actions
        .act(ActionCall {
            id: "set_settings".to_string(),
            args: serde_json::json!({ "compact_model": "nope" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("compact_model"), "{err}");
}

#[test]
fn create_session_and_config_carry_chara_as_a_snapshot() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    let done = faces
        .actions
        .act(ActionCall {
            id: "create_session".to_string(),
            args: serde_json::json!({
                "name": "w-chara",
                "kind": "single",
                "agents": [{
                    "name": "甲",
                    "transient": true,
                    "modules": ["a"],
                    "chara": "建会话时写的人设"
                }],
                "tier": "host"
            }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("建会话");
    let Acted::Done(v) = done else {
        panic!("建会话应同步完成");
    };
    let sid = v["sid"].as_str().expect("sid").to_string();
    let (meta, _) = faces.history.open(&sid).expect("读 meta");
    assert_eq!(meta.agents[0].chara, "建会话时写的人设");
    // 会话配置里的人设是 copy：允许临时改，写回同一份快照。
    faces
        .work
        .edit(
            &sid,
            SessionEdit {
                agents: vec![ConfigAgent {
                    name: "甲".to_string(),
                    modules: vec!["a".to_string()],
                    model: String::new(),
                    chara: "会话里临时改的人设".to_string(),
                }],
                tier: "host".to_string(),
                guest: None,
                pins: std::collections::BTreeMap::new(),
                permission: None,
                shared: None,
            },
        )
        .expect("改配置");
    let (meta, _) = faces.history.open(&sid).expect("读回 meta");
    assert_eq!(meta.agents[0].chara, "会话里临时改的人设");
}

#[test]
fn host_session_uses_package_bin_path() {
    use std::sync::Arc;
    let dir = crate::tests::scratch("host-session-pkg");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("建 bin");
    let mut p = crate::tests::doubles::pkg("pkgtool", "1");
    p.source_dir = dir.clone();
    let packages = Arc::new(crate::tests::doubles::InMemoryPackages::from(vec![p]));
    let probe = Arc::new(crate::tests::doubles::FixedProbe {
        dirs: vec![bin.clone()],
        ..Default::default()
    });
    let mut m = module_with_runtimes("a", &["pkgtool"]);
    m.manifest.tools.insert("run".to_string(), decl("pkgtool"));
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"tool\",\"name\":\"run\",\"args\":{}}".into(),
            "{\"type\":\"say\",\"text\":\"done\"}".into(),
        ],
    );
    let runner = Arc::new(super::super::builders::EnvRecordingRunner::new());
    let mut core = crate::tests::doubles::core_with_pkgs_probe(
        vec![m],
        crate::tests::doubles::gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
        Arc::new(crate::tests::doubles::FakeCatalog::new(vec![
            "m".to_string(),
        ])),
        Arc::new(crate::tests::doubles::InMemoryHistory::new()),
        Arc::new(crate::tests::doubles::InMemorySysIo::new()),
        packages,
        probe,
    );
    let sid = core
        .create_work(crate::tests::doubles::work("w", SINGLE, &["a"]))
        .unwrap()
        .sid;
    crate::tests::doubles::with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "干活",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let paths = runner.paths.lock().expect("锁");
    assert_eq!(paths.len(), 1, "工具真的跑了一次");
    let path = paths[0].as_ref().expect("本机档要注入包 PATH");
    let first = std::env::split_paths(path).next().expect("PATH 第一段");
    assert_eq!(first, bin, "包的 bin 前置到这一趟 PATH");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_permission_profile_action_switches_the_session_profile() {
    use crate::units::permission::api::ProfileId;
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    let (opened, _) = faces
        .work
        .create_work(single_work("w-profile", &["a"]))
        .expect("建会话");
    let sid = opened.sid;
    faces
        .actions
        .act(ActionCall {
            id: "set_permission_profile".to_string(),
            args: serde_json::json!({ "session_id": sid, "profile": "online" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("快速切档");
    assert_eq!(
        faces.work.config(&sid).unwrap().permission.profile,
        ProfileId::Online
    );
    assert!(
        faces
            .actions
            .act(ActionCall {
                id: "set_permission_profile".to_string(),
                args: serde_json::json!({ "session_id": sid, "profile": "nope" }),
                caller: Caller::User,
                out: Output::Final,
            })
            .is_err(),
        "未知档位如实拒绝"
    );
}

#[test]
fn set_thinking_action_switches_the_work_level_and_rejects_unmapped_levels() {
    use crate::units::catalog::registry::api::ThinkingSpec;
    use std::collections::BTreeMap;
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    faces
        .registry
        .upsert_model(crate::units::catalog::registry::api::ModelDraft {
            thinking: ThinkingSpec {
                default: None,
                levels: BTreeMap::from([
                    ("low".to_string(), serde_json::json!("low")),
                    ("high".to_string(), serde_json::json!("high")),
                ]),
            },
            ..model_draft("m", "M", "m", "p", "", 32_000)
        })
        .unwrap();
    let (opened, _) = faces
        .work
        .create_work(single_work("w-thinking", &["a"]))
        .expect("建会话");
    let sid = opened.sid;
    faces
        .actions
        .act(ActionCall {
            id: "set_thinking".to_string(),
            args: serde_json::json!({ "session_id": sid, "level": "high" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("切思考档");
    let cfg = faces.work.config(&sid).expect("读配置");
    assert_eq!(cfg.thinking.level.as_deref(), Some("high"));
    assert!(
        cfg.thinking.options.iter().any(|o| o.id == "high"),
        "已登记的档要列出来"
    );
    assert!(
        !cfg.thinking.options.iter().any(|o| o.id == "max"),
        "没登记的档不列出来"
    );
    assert_eq!(
        cfg.thinking.default_level.as_deref(),
        None,
        "没有统一默认档"
    );
    assert_eq!(cfg.thinking.label, "high", "有会话选择时显示标签");
    // 没登记的档：拒绝并列出缺映射的模型（不静默降级）
    let err = faces
        .actions
        .act(ActionCall {
            id: "set_thinking".to_string(),
            args: serde_json::json!({ "session_id": sid, "level": "max" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("模型 m") || err.contains("m"), "{}", err);
    // 空串 = 随模型默认
    faces
        .actions
        .act(ActionCall {
            id: "set_thinking".to_string(),
            args: serde_json::json!({ "session_id": sid, "level": "" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .unwrap();
    let after = faces.work.config(&sid).unwrap();
    assert_eq!(after.thinking.level.as_deref(), None, "空串回到跟随默认");
    assert_eq!(
        after.thinking.default_level.as_deref(),
        None,
        "模型没有默认档"
    );
    assert_eq!(after.thinking.label, "未设置", "没有选择也没有默认");
}

#[test]
fn set_context_freshness_action_flips_the_work_switch_and_reports_it() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    let (opened, _) = faces
        .work
        .create_work(single_work("w-fresh", &["a"]))
        .expect("建会话");
    let sid = opened.sid;
    let view = |faces: &Faces| {
        faces
            .work
            .session_views(&faces.history.list().expect("列历史"))
            .expect("会话视图")
            .into_iter()
            .find(|v| v.sid == sid)
            .expect("找到会话")
    };
    assert!(
        !view(&faces).context_freshness,
        "默认关：只记读事实、不改发送视图"
    );
    assert_eq!(view(&faces).freshness_keep_percent, 80, "默认保底 80%");
    let flip = |on: bool, keep: Option<u8>| {
        let mut args = serde_json::json!({ "session_id": sid, "on": on });
        if let Some(keep) = keep {
            args["keep_percent"] = serde_json::json!(keep);
        }
        faces.actions.act(ActionCall {
            id: "set_context_freshness".to_string(),
            args,
            caller: Caller::User,
            out: Output::Final,
        })
    };
    flip(true, Some(70)).expect("开开关并设保底");
    // 切开关会把子树会话从内存移除（与思考档一致），所以查**落盘事实**而不是视图。
    let meta = faces.history.open(&sid).expect("读 meta").0;
    assert!(meta.context_freshness, "开关写到工作根 meta 上");
    assert_eq!(meta.freshness_keep_percent, 70, "保底也写在工作根 meta 上");
    // 关开关不带百分比：百分比保持不变（下次再开会沿用它）。
    flip(false, None).expect("关开关");
    let meta = faces.history.open(&sid).expect("读 meta").0;
    assert!(!meta.context_freshness);
    assert_eq!(meta.freshness_keep_percent, 70, "不带百分比时沿用现值");
    // 100% 等于永不替换，与关闭无异：只接受 0..=90。
    let err = flip(true, Some(91)).expect_err("超过 90% 要拒绝");
    assert!(err.contains("90%"), "错误要说清上限：{err}");
    assert!(
        !faces
            .history
            .open(&sid)
            .expect("读 meta")
            .0
            .context_freshness,
        "拒绝后不改落盘事实"
    );
}

#[test]
fn create_work_takes_the_context_freshness_chosen_in_the_wizard() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    let opened = faces
        .work
        .create_work(WorkSpec {
            freshness: Some((true, 60)),
            ..single_work("w-fresh-wizard", &["a"])
        })
        .expect("建会话");
    let sid = opened.0.sid;
    let meta = faces.history.open(&sid).expect("读 meta").0;
    assert!(meta.context_freshness, "创建时勾选就落盘");
    assert_eq!(meta.freshness_keep_percent, 60, "创建时的保底也落盘");
    let view = faces
        .work
        .session_views(&faces.history.list().expect("列历史"))
        .expect("会话视图")
        .into_iter()
        .find(|v| v.sid == sid)
        .expect("找到会话");
    assert!(view.context_freshness, "视图回报创建时的开关");
    assert_eq!(view.freshness_keep_percent, 60, "视图回报创建时的保底");
}

#[test]
fn thinking_view_uses_the_common_default_and_hides_unmapped_levels() {
    use crate::units::catalog::registry::api::ThinkingSpec;
    use std::collections::BTreeMap;
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    faces
        .registry
        .upsert_model(crate::units::catalog::registry::api::ModelDraft {
            thinking: ThinkingSpec {
                default: Some("high".to_string()),
                levels: BTreeMap::from([
                    ("low".to_string(), serde_json::json!("low")),
                    ("high".to_string(), serde_json::json!("high")),
                ]),
            },
            ..model_draft("m", "M", "m", "p", "", 32_000)
        })
        .unwrap();
    let (opened, _) = faces
        .work
        .create_work(single_work("w-default", &["a"]))
        .expect("建会话");
    let cfg = faces.work.config(&opened.sid).expect("读配置");
    assert_eq!(cfg.thinking.level, None, "没选过档");
    assert_eq!(
        cfg.thinking.default_level.as_deref(),
        Some("high"),
        "这组模型共同的默认档"
    );
    assert_eq!(cfg.thinking.label, "high", "默认档直接作为当前标签显示");
    let ids: Vec<&str> = cfg.thinking.options.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["low", "high"], "只列已映射的档（按目录顺序）");
    assert!(
        !cfg.thinking.options.iter().any(|o| o.id == "max"),
        "没映射的档不列出来"
    );
}
