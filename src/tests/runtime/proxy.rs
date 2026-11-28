//! 目的：钉 core_proxy 六工具的声明、授权、幂等、部分成功与游标契约。
//! 管：六工具声明与授权、无授权不达宿主、授权范围过期与边界、create_session 幂等、send 部分成功、observe 无正文、分页、control 幂等、声明形态、真实 bridge。
//! 不管：HTTP 路由（routes.rs）与讨论发言规则。
//! 联动：`proxy::api` 的 `ProxyHost` / `ProxyTools` / `ProxyHandler`、FakeProxyHost、runtime 的 `service/proxy.rs`（`ProxyBridge`）。

use super::super::prelude::*;
use crate::presentation::faces::Faces;
use crate::tests::builders::decl_of;

use crate::units::business::proxy::api as d;
use crate::units::business::proxy::api::ProxyTools;
use crate::units::business::session_model::api::{PROXY, SINGLE};

fn rig() -> (Arc<FakeProxyHost>, ProxyTools) {
    let host = Arc::new(FakeProxyHost::new());
    let tools = ProxyTools::new(host.clone(), test_systools().tools);
    (host, tools)
}

fn full_grant() -> d::Grant {
    d::Grant {
        tools: vec![
            d::CATALOG.to_string(),
            d::CREATE.to_string(),
            d::SEND.to_string(),
            d::OBSERVE.to_string(),
            d::CONTROL.to_string(),
            d::MESSAGES.to_string(),
        ],
        ..Default::default()
    }
}

fn ctx(grant: Option<d::Grant>) -> d::ProxyCall {
    d::ProxyCall {
        grant,
        parent: Some("main".to_string()),
        now: 1000,
    }
}

/// 目的：六个工具必须真的在总表里，且只挂在代理会话类型的 `system_tools` 上（越权防线出自会话形状）。
#[test]
pub(crate) fn the_six_tools_are_declared_and_granted_to_core_proxy() {
    let st = test_systools();
    const SIX: [&str; 6] = [
        d::CATALOG,
        d::CREATE,
        d::SEND,
        d::OBSERVE,
        d::CONTROL,
        d::MESSAGES,
    ];
    for name in SIX {
        let schema = st.tools.get(name).expect("代理工具必须在总表里");
        assert!(!schema.desc.trim().is_empty(), "{} 缺说明", name);
        assert!(schema.params.is_some(), "{} 缺参数契约", name);
    }
    // 会话形状：只有代理类型把六个挂进 `system_tools`（其余类型的 system 面为空）。
    let decls = crate::tests::doubles::test_decls();
    let proxy = decls
        .spec_of(crate::units::business::session_model::api::PROXY)
        .expect("代理类型在表里");
    for name in SIX {
        assert!(
            proxy.system_tools.iter().any(|t| t == name),
            "代理类型该声明 system 工具 {}：{:?}",
            name,
            proxy.system_tools
        );
    }
    for kind in [
        crate::units::business::session_model::api::SINGLE,
        crate::units::business::session_model::api::MEETING,
    ] {
        let spec = decls.spec_of(kind).expect("产品类型在表里");
        for name in SIX {
            assert!(
                !spec.system_tools.iter().any(|t| t == name),
                "{} 不该声明代理工具 {}",
                kind,
                name
            );
        }
    }
    // 角色位不再含这些名字：它们来自会话类型，不来自流程段。
    let core = decls
        .loops()
        .cast_of("proxy", "core_proxy")
        .expect("代理段的核心角色位在册");
    assert!(
        core.tools.is_empty(),
        "代理角色位的工具面已交给会话类型：{:?}",
        core.tools
    );
    assert!(
        !st.tools.contains_key("core_proxy"),
        "core_proxy 是席位，不是工具"
    );
}

/// 目的：没有授权就什么都不做：六个工具全部如实拒绝，且**根本没碰宿主**。
#[test]
pub(crate) fn without_a_grant_nothing_reaches_the_host() {
    let (host, mut tools) = rig();
    let c = ctx(None);
    for (name, args) in [
        (d::CATALOG, r#"{"scope":"all"}"#),
        (
            d::CREATE,
            r#"{"kind":"single","agents":[{"ref":"a"}],"task":"做事","request_id":"r1"}"#,
        ),
        (
            d::SEND,
            r#"{"targets":["t1"],"message":"好","kind":"task","request_id":"r2"}"#,
        ),
        (d::OBSERVE, r#"{"session_id":"s1","view":"status"}"#),
        (d::MESSAGES, r#"{"session_id":"s1","from":0,"count":5}"#),
        (
            d::CONTROL,
            r#"{"session_id":"s1","action":"stop","reason":"停","request_id":"r3"}"#,
        ),
    ] {
        let out = tools.call(&c, name, args);
        assert!(!out.ok, "{} 没有授权必须拒绝", name);
        assert!(
            out.output.contains("没有代理授权"),
            "{}：{}",
            name,
            out.output
        );
    }
    assert!(
        host.calls().is_empty(),
        "未授权不允许碰宿主：{:?}",
        host.calls()
    );
}

/// 目的：授权边界：工具、有效期、模型与会话四道都算数。
#[test]
pub(crate) fn grant_scope_expiry_and_bounds_are_enforced() {
    let (host, mut tools) = rig();
    let c = ctx(Some(d::Grant {
        tools: vec![d::CATALOG.to_string()],
        ..Default::default()
    }));
    let out = tools.call(
        &c,
        d::CREATE,
        r#"{"kind":"single","agents":[{"ref":"a"}],"task":"做事","request_id":"r1"}"#,
    );
    assert!(
        !out.ok && out.output.contains("不包含这个工具"),
        "{}",
        out.output
    );
    assert!(host.calls().is_empty(), "越范围不得碰宿主");

    let expired = ctx(Some(d::Grant {
        tools: vec![d::CATALOG.to_string()],
        expires_at: Some(1),
        ..Default::default()
    }));
    let out = tools.call(&expired, d::CATALOG, r#"{"scope":"all"}"#);
    assert!(!out.ok && out.output.contains("已过期"), "{}", out.output);
    assert!(host.calls().is_empty());

    let model_scoped = ctx(Some(d::Grant {
        tools: vec![d::CREATE.to_string()],
        models: vec!["gpt".to_string()],
        ..Default::default()
    }));
    let out = tools.call(
        &model_scoped,
        d::CREATE,
        r#"{"kind":"single","agents":[{"name":"x","modules":["m1"],"model":"other"}],"task":"做事","request_id":"r2"}"#,
    );
    assert!(
        !out.ok && out.output.contains("不覆盖这个模型"),
        "{}",
        out.output
    );
    assert!(host.created().is_empty(), "模型越界不得建会话");

    let sess_scoped = ctx(Some(d::Grant {
        tools: vec![d::OBSERVE.to_string()],
        sessions: vec!["s1".to_string()],
        ..Default::default()
    }));
    let out = tools.call(
        &sess_scoped,
        d::OBSERVE,
        r#"{"session_id":"s9","view":"status"}"#,
    );
    assert!(
        !out.ok && out.output.contains("不覆盖这个会话"),
        "{}",
        out.output
    );
}

/// 目的：create_session：先与登记处事实核对，再建立；失败不留半成品，重放不重复创建。
#[test]
pub(crate) fn create_session_validates_then_creates_once() {
    let (host, mut tools) = rig();
    let c = ctx(Some(full_grant()));
    let args = r#"{"kind":"single","agents":[{"name":"x","modules":["m1"],"model":"gpt"}],"task":"做事","request_id":"r1"}"#;
    let out = tools.call(&c, d::CREATE, args);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("work-r1"), "{}", out.output);
    let made = host.created();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].task, "做事");
    assert_eq!(made[0].agents[0].name, "x");
    assert!(made[0].agents[0].transient, "新组装的 agent 是临时项");

    let again = tools.call(&c, d::CREATE, args);
    assert_eq!(again, out, "重放回同一结果");
    assert_eq!(host.created().len(), 1, "重放不得重复创建");

    for (bad, why) in [
        (
            r#"{"kind":"single","agents":[{"name":"x","modules":["nope"]}],"task":"做事","request_id":"b1"}"#,
            "无此模块",
        ),
        (
            r#"{"kind":"single","agents":[{"ref":"nope"}],"task":"做事","request_id":"b5"}"#,
            "不在登记处",
        ),
        (
            r#"{"kind":"single","agents":[{"name":"a/b","modules":["m1"]}],"task":"做事","request_id":"b6"}"#,
            "名字不合法",
        ),
        (
            r#"{"kind":"single","agents":[{"name":"x","modules":["m1"],"extra":1}],"task":"做事","request_id":"b7"}"#,
            "不认识的键",
        ),
        (
            r#"{"kind":"single","agents":[{"name":"x","modules":["m1"]}],"request_id":"b8"}"#,
            "task",
        ),
    ] {
        let out = tools.call(&c, d::CREATE, bad);
        assert!(!out.ok, "{} 必须被拒：{}", why, out.output);
        assert!(
            out.output.contains(why),
            "{} 的理由要如实：{}",
            why,
            out.output
        );
    }
    assert_eq!(host.created().len(), 1, "被拒的创建不得留下半成品");

    // 宿主失败不记账：重放同一个 request_id 会再试，而不是把失败当成已完成。
    host.fail_create("磁盘满");
    let out = tools.call(
        &c,
        d::CREATE,
        r#"{"kind":"single","agents":[{"ref":"a"}],"task":"做事","request_id":"r9"}"#,
    );
    assert!(!out.ok && out.output.contains("磁盘满"), "{}", out.output);
    assert_eq!(host.created().len(), 1);
    host.clear_fail_create();
    let out = tools.call(
        &c,
        d::CREATE,
        r#"{"kind":"single","agents":[{"ref":"a"}],"task":"做事","request_id":"r9"}"#,
    );
    assert!(out.ok, "失败不记账，重放要再试：{}", out.output);
    assert_eq!(host.created().len(), 2);
}

/// 目的：send_session_message：来源如实标记、多目标部分成功、代答必须带独立引用。
#[test]
pub(crate) fn send_marks_source_and_reports_partial_success() {
    let (host, mut tools) = rig();
    host.fail_send("t2");
    let c = ctx(Some(full_grant()));
    let out = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["t1","t2"],"message":"继续","kind":"task","request_id":"r1"}"#,
    );
    assert!(!out.ok, "有目标失败就不能报成功：{}", out.output);
    assert!(
        out.output.contains("t1") && out.output.contains("t2"),
        "成功与失败要分别可见：{}",
        out.output
    );
    assert_eq!(
        host.relayed().last().map(|m| m.kind),
        Some(d::MessageKind::Task),
        "转达的种类要如实记录"
    );

    let calls = host.calls().len();
    let again = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["t1","t2"],"message":"继续","kind":"task","request_id":"r1"}"#,
    );
    assert_eq!(again, out, "重放回同一结果（含部分失败）");
    assert_eq!(host.calls().len(), calls, "重放不再动宿主");

    // 代答只按 kind 记一条来源记录。
    let out = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["t1"],"message":"好","kind":"user_reply","request_id":"r3"}"#,
    );
    assert!(out.ok, "{}", out.output);
    let last = host.relayed().pop().expect("有转达记录");
    assert_eq!(last.kind, d::MessageKind::UserReply);
    assert_eq!(last.text, "好");
}

/// 目的：observe_session：只读、不回消息正文、不推进；非法 view 不碰宿主。
#[test]
pub(crate) fn observe_reports_no_message_bodies() {
    let (host, mut tools) = rig();
    host.add_event("e1");
    host.add_event("e2");
    let c = ctx(Some(full_grant()));
    let out = tools.call(&c, d::OBSERVE, r#"{"session_id":"s1","view":"status"}"#);
    assert!(out.ok, "{}", out.output);
    let v: serde_json::Value = serde_json::from_str(&out.output).expect("回执是 JSON");
    assert_eq!(v["message_count"], 2, "{}", out.output);
    assert!(
        !out.output.contains("e1") && !out.output.contains("e2"),
        "观察不得回消息正文（正文走倒查）：{}",
        out.output
    );
    // `latest` 这一面已取消：正文一律经 read_session_messages 倒查。
    let calls = host.calls().len();
    let bad = tools.call(&c, d::OBSERVE, r#"{"session_id":"s1","view":"latest"}"#);
    assert!(!bad.ok && bad.output.contains("view"), "{}", bad.output);
    let bad = tools.call(&c, d::OBSERVE, r#"{"session_id":"s1","view":"nope"}"#);
    assert!(!bad.ok && bad.output.contains("view"), "{}", bad.output);
    assert_eq!(host.calls().len(), calls, "非法 view 不得碰宿主");
}

/// 目的：read_session_messages：0 = 最新一条，按新→旧，可继续往更早翻。
#[test]
pub(crate) fn read_session_messages_pages_newest_first() {
    let (host, mut tools) = rig();
    host.add_event("e1");
    host.add_event("e2");
    host.add_event("e3");
    let c = ctx(Some(full_grant()));
    let first = tools.call(&c, d::MESSAGES, r#"{"session_id":"s1","from":0,"count":2}"#);
    assert!(first.ok, "{}", first.output);
    let v: serde_json::Value = serde_json::from_str(&first.output).expect("回执是 JSON");
    assert_eq!(
        v["messages"][0]["text"], "e3",
        "最新一条在前：{}",
        first.output
    );
    assert_eq!(v["messages"][1]["text"], "e2", "再往旧：{}", first.output);
    assert_eq!(v["next"], 2, "还有更早的：{}", first.output);

    let second = tools.call(&c, d::MESSAGES, r#"{"session_id":"s1","from":2,"count":2}"#);
    assert!(second.ok, "{}", second.output);
    let w: serde_json::Value = serde_json::from_str(&second.output).expect("回执是 JSON");
    assert_eq!(w["messages"][0]["text"], "e1", "{}", second.output);
    assert!(w["next"].is_null(), "到底了不该有 next：{}", second.output);
}

/// 目的：control_session：回执是实际状态、重放幂等、宿主拒绝照实回报。
#[test]
pub(crate) fn control_is_idempotent_and_reports_the_actual_state() {
    let (host, mut tools) = rig();
    let c = ctx(Some(full_grant()));
    let args = r#"{"session_id":"s1","action":"stop","reason":"用户要求","request_id":"r1"}"#;
    let out = tools.call(&c, d::CONTROL, args);
    assert!(out.ok, "{}", out.output);
    assert!(
        out.output.contains("\"state\":\"stopped\""),
        "{}",
        out.output
    );

    let calls = host.calls().len();
    let again = tools.call(&c, d::CONTROL, args);
    assert_eq!(again, out, "重放回同一结果");
    assert_eq!(host.calls().len(), calls, "重放不再动宿主");

    host.fail_control("archive");
    let bad = tools.call(
        &c,
        d::CONTROL,
        r#"{"session_id":"s1","action":"archive","reason":"收尾","request_id":"r2"}"#,
    );
    assert!(
        !bad.ok && bad.output.contains("不能 archive"),
        "{}",
        bad.output
    );

    let bad = tools.call(
        &c,
        d::CONTROL,
        r#"{"session_id":"s1","action":"explode","reason":"x","request_id":"r3"}"#,
    );
    assert!(!bad.ok && bad.output.contains("action"), "{}", bad.output);
}

/// 目的：声明层先挡形状（没写的键、类型不对），语义层再挡取值；都不是代理工具的直接拒绝。
#[test]
pub(crate) fn declaration_shape_is_enforced_before_semantics() {
    let (host, mut tools) = rig();
    let c = ctx(Some(full_grant()));
    let out = tools.call(&c, d::CATALOG, r#"{"scope":"all","nope":1}"#);
    assert!(!out.ok, "声明里没写的键必须拒收：{}", out.output);
    let out = tools.call(&c, d::CATALOG, r#"{"scope":"everything"}"#);
    assert!(!out.ok && out.output.contains("scope"), "{}", out.output);
    let out = tools.call(&c, "read", r#"{"path":"/x"}"#);
    assert!(
        !out.ok && out.output.contains("不是代理工具"),
        "{}",
        out.output
    );
    assert!(
        host.calls().is_empty(),
        "形状/语义不过的调用不得碰宿主：{:?}",
        host.calls()
    );
}
/// 目的：代理会话跑一轮真实生成：模型请求 `catalog_agents` → 经 handler / 队列桥在核心线程执行 →
///   工具行落进它自己的转录。这条钉住“代理会话就是普通成员会话 + 派发去特例”的整条链。
#[test]
pub(crate) fn the_proxy_session_runs_a_turn_through_the_generic_loop() {
    use crate::units::business::turn::turn::api::Output;

    let (handle, faces) = super::super::ops_with(
        vec![],
        vec![
            r#"{"type":"tool","name":"catalog_agents","args":{"scope":"agents"}}"#,
            "好",
        ],
    );
    let sid = handle
        .call(|core| core.create_proxy("w-proxy", 7))
        .expect("建代理会话");
    faces.turn.say(&sid, "开始", Output::Final).expect("跑一轮");

    let (meta, events) = faces.history.open(&sid).expect("打开代理会话");
    assert_eq!(meta.spec.kind, PROXY);
    assert!(meta.delegation.is_some(), "代理会话带全权委托");
    let text = serde_json::to_string(&events).expect("转录");
    assert!(
        text.contains("catalog_agents"),
        "工具行该落进转录：{}",
        text
    );
}
/// 目的：子会话停下 → 通知代理（只写一条通知行，**不转发子会话转录**）。
#[test]
pub(crate) fn a_finished_child_notifies_the_proxy_without_dumping_its_transcript() {
    use crate::units::business::catalog::modules::api::ModuleManifest;
    let module = Module {
        manifest: ModuleManifest {
            build: None,
            id: "m1".to_string(),
            version: "0.1.0".to_string(),
            brief: "测试模块".to_string(),
            system: "用法：测试。".to_string(),
            enabled: true,
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
            services: Default::default(),
            secrets: Default::default(),
        },
        root: PathBuf::from("modules").join("m1"),
        private: None,
    };
    let (handle, faces) = super::super::ops_with(vec![module], vec![]);
    let proxy = handle
        .call(|core| core.create_proxy("w-notify", 1))
        .expect("建代理会话");
    let child = handle
        .call(|core| {
            let spec = d::NewSession {
                freshness: None,
                kind: SINGLE.to_string(),
                agents: vec![AgentMeta {
                    name: "c1".to_string(),
                    transient: true,
                    modules: vec!["m1".to_string()],
                    model: None,
                    chara: String::new(),

                    abilities: None,
                }],
                task: "做事".to_string(),
                request_id: "r1".to_string(),
                name: None,
                sandbox: None,
                parent: Some("w-notify".to_string()),
                permission: None,
                workspace: None,
            };
            Ok(core.create_session(&spec)?.0.session)
        })
        .expect("建子工作");
    assert_eq!(
        child, "w-notify--c1",
        "子会话名 = 父名 -- 席位名（不另起时间戳）"
    );
    let notified = handle
        .call({
            let c = child.clone();
            move |core| Ok(core.turn.notify_proxy_of_child(&c))
        })
        .expect("通知");
    assert_eq!(notified.as_deref(), Some(proxy.as_str()));
    let (_, events) = faces.history.open(&proxy).expect("代理转录");
    let text = serde_json::to_string(&events).expect("JSON");
    assert!(text.contains("子会话"), "通知行该落进代理会话：{}", text);
    assert!(!text.contains("做事"), "不得把子会话的转录灌进来：{}", text);
}

/// 目的：代理会话的身份是**席位提示词**（core_proxy）+ 环境 + 调用约定，不是普通 agent 身份；
///   反过来，普通 agent 会话也不会沾上代理席的话。
#[test]
pub(crate) fn the_proxy_identity_is_the_role_prompt() {
    use crate::units::business::catalog::modules::api::ModuleManifest;

    let module = Module {
        manifest: ModuleManifest {
            build: None,
            id: "m1".to_string(),
            version: "0.1.0".to_string(),
            brief: "测试模块".to_string(),
            system: "用法：测试。".to_string(),
            enabled: true,
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
            services: Default::default(),
            secrets: Default::default(),
        },
        root: PathBuf::from("modules").join("m1"),
        private: None,
    };
    let (handle, faces) = super::super::ops_with(vec![module], vec![]);
    let proxy = handle
        .call(|core| core.create_proxy("w-role", 1))
        .expect("建代理会话");
    let identity = handle
        .call({
            let s = proxy.clone();
            move |core| Ok(core.single_identity(&s))
        })
        .expect("问身份")
        .expect("代理会话在表里");
    assert!(
        identity.contains("你是核心代理"),
        "席位提示词没进去：{}",
        identity
    );
    assert!(
        identity.contains("派完活就让出回合"),
        "代理这一类会话的机制说明要进身份块：{}",
        identity
    );
    assert!(identity.contains("w-role"), "环境块不能缺席：{}", identity);
    assert!(
        identity.contains("工具调用约定："),
        "调用约定不能缺席（少了模型不知道能调工具）：{}",
        identity
    );

    // 普通 agent 会话：环境与调用约定同一份口径，但不含代理席的话。
    let (work, _) = faces
        .work
        .create_work(super::super::single_work("w-role-plain", &["m1"]))
        .expect("建工作");
    let plain = handle
        .call({
            let s = work.sid.clone();
            move |core| Ok(core.single_identity(&s))
        })
        .expect("问身份")
        .expect("普通会话在表里");
    assert!(plain.contains("w-role-plain"), "{}", plain);
    assert!(plain.contains("工具调用约定："), "{}", plain);
    assert!(
        !plain.contains("你是核心代理"),
        "普通会话不该有代理席：{}",
        plain
    );
    // 机制说明按**这个会话的类别**取：单 agent 工作拿单 agent 那一份，不沾代理那份。
    assert!(plain.contains("单 agent 工作"), "{}", plain);
    assert!(!plain.contains("派完活就让出回合"), "{}", plain);
}

/// 目的：用户入口的第三人形态：`create_work(PROXY)` 建出**代理会话**——
///   没有名单、带全权委托、运行态正常；名单 / 需求与形态不符时整条拒绝、不留半成品。
#[test]
pub(crate) fn create_work_with_proxy_mode_makes_a_delegated_session() {
    let (handle, faces) = super::super::ops_with(vec![], vec![]);
    let spec = crate::units::business::work::api::WorkSpec {
        freshness: None,
        name: "w-user-proxy".to_string(),
        kind: PROXY.to_string(),
        agents: Vec::new(),
        task: None,
        delegate: false,
        permission: None,
        sandbox: decl_of(Side::Host, None),
        workspace: None,
    };
    let (opened, _head) = faces.work.create_work(spec.clone()).expect("建代理会话");
    assert!(opened.agents.is_empty(), "代理会话没有名单");
    let (meta, _) = faces.history.open(&opened.sid).expect("落盘");
    assert_eq!(meta.spec.kind, PROXY);
    assert!(meta.delegation.is_some(), "选代理形态 = 授予全权");
    assert!(meta.agents.is_empty() && meta.parent.is_none());
    assert_eq!(meta.run, RunState::Active);
    // 代理会话在表里、身份是 core_proxy 面（与 create_proxy 同一条装配）。
    let identity = handle
        .call({
            let s = opened.sid.clone();
            move |core| Ok(core.single_identity(&s))
        })
        .expect("问身份")
        .expect("代理会话在表里");
    assert!(identity.contains("你是核心代理"), "{}", identity);
    // 读模型：形态与运行态都如实给出（前端据此渲染代理会话、标出暂停 / 关闭）。
    let views = faces
        .work
        .session_views(&faces.history.list().expect("列表"))
        .expect("视图");
    let v = views
        .iter()
        .find(|v| v.sid == opened.sid)
        .expect("代理会话在列表里");
    assert_eq!(v.kind, PROXY);
    assert_eq!(v.run, "active");
    assert!(!v.can_update_task, "代理会话没有本次需求");

    // 与形态不符的载荷：整条拒绝，不留半成品。
    let mut bad = spec.clone();
    bad.name = "w-user-proxy-2".to_string();
    bad.agents = vec![crate::units::pure::registry_model::api::AgentMeta {
        name: "a".to_string(),
        transient: true,
        modules: vec!["m1".to_string()],
        model: None,
        chara: String::new(),

        abilities: None,
    }];
    assert!(
        faces
            .work
            .create_work(bad)
            .unwrap_err()
            .contains("不接受 agent 名单")
    );
    assert!(
        faces.history.open("w-user-proxy-2").is_err(),
        "拒绝后不留半成品"
    );

    // 档位与其余形态同一把尺子：用户选虚拟机档就按它建（承载不了则如实拒绝、什么都不留）。
    let mut vm = spec;
    vm.name = "w-user-proxy-vm".to_string();
    let guest = crate::tests::test_guest("proxy-vm");
    vm.sandbox = crate::tests::builders::decl_of(Side::Guest, Some(&guest));
    match faces.work.create_work(vm) {
        Ok((o, _)) => {
            let (m, _) = faces.history.open(&o.sid).expect("落盘");
            assert_eq!(
                m.facts.side,
                Side::Guest,
                "用户选的执行环境要落盘（子工作继承它）"
            );
        }
        Err(e) => {
            assert!(!e.trim().is_empty(), "拒绝要把原因说清");
            assert!(
                faces.history.open("w-user-proxy-vm").is_err(),
                "拒绝后不留半成品"
            );
        }
    }
}

/// 目的：代理会话是 `Session::Single`，有一条持续对话，所以也要显示上下文用量（窗口取核心默认模型）。
#[test]
pub(crate) fn proxy_session_exposes_context_usage() {
    use crate::units::business::llm::api::TokenUsage;
    let gateway = ScriptGateway::new(
        BTreeMap::new(),
        vec!["{\"type\":\"say\",\"text\":\"代理在\"}".to_string()],
    )
    .with_usage(TokenUsage::new(700, 50, Some(750)));
    let handle =
        crate::units::business::runtime::api::Handle::spawn(core_with_gateway(vec![], gateway))
            .expect("起核心线程");
    let faces = Faces::from_handle(&handle);
    let spec = crate::units::business::work::api::WorkSpec {
        freshness: None,
        name: "w-proxy-ctx".to_string(),
        kind: PROXY.to_string(),
        agents: Vec::new(),
        task: None,
        delegate: false,
        permission: None,
        sandbox: decl_of(Side::Host, None),
        workspace: None,
    };
    let sid = faces.work.create_work(spec).expect("建代理会话").0.sid;
    faces
        .turn
        .say(
            &sid,
            "开始",
            crate::units::business::turn::turn::api::Output::Final,
        )
        .expect("代理一轮");
    let views = faces
        .work
        .session_views(&faces.history.list().expect("列表"))
        .expect("视图");
    let view = views
        .iter()
        .find(|v| v.sid == sid)
        .expect("代理会话在视图里");
    let ctx = view.context.expect("代理会话也要有上下文用量");
    assert_eq!(ctx.total_tokens, 750);
    assert!(ctx.exact);
    assert!(ctx.window > 0, "窗口取核心默认模型");
}

/// 目的：核心代理建子会话时不能自选环境：给了 tier / guest 也整段继承主会话。
#[test]
pub(crate) fn ai_created_child_inherits_the_proxy_environment() {
    let (handle, faces) = super::super::ops_with(vec![module_of("m1")], vec![]);
    let proxy = handle
        .call(|core| core.create_proxy("p-env", 1))
        .expect("建代理");
    let child = handle
        .call({
            let proxy = proxy.clone();
            move |core| {
                let spec = d::NewSession {
                    freshness: None,
                    kind: SINGLE.to_string(),
                    agents: vec![AgentMeta {
                        name: "a".to_string(),
                        transient: true,
                        modules: vec!["m1".to_string()],
                        model: None,
                        chara: String::new(),

                        abilities: None,
                    }],
                    task: "做事".to_string(),
                    request_id: "r-env".to_string(),
                    name: None,
                    sandbox: Some(crate::units::business::sandbox::api::SandboxDecl {
                        policy: "vm".to_string(),
                        params: serde_json::json!({ "guest": "nope" }),
                        pins: Default::default(),
                    }),
                    parent: Some(proxy),
                    permission: None,
                    workspace: None,
                };
                Ok(core.create_session(&spec)?.0.session)
            }
        })
        .expect("建子会话");
    let (meta, _) = faces.history.open(&child).expect("子会话落盘");
    assert_eq!(meta.facts.side, Side::Host, "子会话环境随代理主会话");
    assert_eq!(meta.facts.guest, None, "AI 不能给子会话选虚拟根");
}

/// 目的：代理也能从状态栏快速切档；不经过 edit_session（代理没有名单）。
#[test]
pub(crate) fn proxy_profile_can_be_quick_switched() {
    use crate::units::pure::permission::api::ProfileId;
    let (handle, faces) = super::super::ops_with(vec![], vec!["好"]);
    let sid = handle
        .call(|core| core.create_proxy("w-switch", 1))
        .expect("建代理");
    let target = sid.clone();
    handle
        .call(move |core| core.set_permission_profile(&target, ProfileId::Offline))
        .expect("代理快速切档");
    let (meta, _) = faces.history.open(&sid).expect("落盘");
    assert_eq!(
        meta.permission.ceiling.as_ref().and_then(|l| l.profile),
        Some(ProfileId::Offline)
    );
    assert_eq!(
        faces.work.config(&sid).unwrap().permission.profile,
        ProfileId::Offline
    );
}
