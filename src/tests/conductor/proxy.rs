//! 核心代理工具（core_proxy）的契约测试：声明、授权、幂等、部分成功与游标。
//! 真实会话宿主尚未落地：动作经 conductor::ports::ProxyHost，测试用 FakeProxyHost 顶替。
use super::super::prelude::*;

use crate::capabilities::conductor::domain::proxy as d;
use crate::capabilities::conductor::service::proxy::ProxyTools;

fn rig() -> (Arc<FakeProxyHost>, ProxyTools) {
    let host = Arc::new(FakeProxyHost::new());
    let tools = ProxyTools::new(host.clone(), test_systools().tools, test_prompts().tools());
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
        source: d::Source::CoreProxy,
        grant,
        parent: Some("main".to_string()),
        now: 1000,
    }
}

/// 六个工具必须真的在总表里、且只发给 core_proxy（越权防线出自身份）。
#[test]
pub(crate) fn the_six_tools_are_declared_and_granted_to_core_proxy() {
    let st = test_systools();
    for name in [
        d::CATALOG,
        d::CREATE,
        d::SEND,
        d::OBSERVE,
        d::CONTROL,
        d::MESSAGES,
    ] {
        let schema = st.tools.get(name).expect("代理工具必须在总表里");
        assert_eq!(
            schema.capability, "none",
            "代理工具不碰文件，capability 必须是 none"
        );
        assert!(!schema.desc.trim().is_empty(), "{} 缺说明", name);
        assert!(schema.params.is_some(), "{} 缺参数契约", name);
    }
    let (face, with_modules) = st.role_face("core_proxy");
    for name in [
        d::CATALOG,
        d::CREATE,
        d::SEND,
        d::OBSERVE,
        d::CONTROL,
        d::MESSAGES,
        "read",
        "list",
        "search",
    ] {
        assert!(face.iter().any(|t| t == name), "core_proxy 该能用 {}", name);
    }
    assert!(
        !with_modules,
        "代理工具代用户决定，但不替 agent 干活（module_tools=false）"
    );
    assert!(
        !st.tools.contains_key("core_proxy"),
        "core_proxy 是角色，不是工具"
    );
    for role in ["discussant", "solo", "executor", "planner", "orchestrator"] {
        let (rface, _) = st.role_face(role);
        for name in [
            d::CATALOG,
            d::CREATE,
            d::SEND,
            d::OBSERVE,
            d::CONTROL,
            d::MESSAGES,
        ] {
            assert!(
                !rface.iter().any(|t| t == name),
                "{} 不该拿到代理工具 {}",
                role,
                name
            );
        }
    }
}

/// 没有授权就什么都不做：五个工具全部如实拒绝，且**根本没碰宿主**。
#[test]
pub(crate) fn without_a_grant_nothing_reaches_the_host() {
    let (host, mut tools) = rig();
    let c = ctx(None);
    for (name, args) in [
        (d::CATALOG, r#"{"scope":"all"}"#),
        (
            d::CREATE,
            r#"{"mode":"single","agents":[{"ref":"a","objective":"做事"}],"request_id":"r1"}"#,
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

/// 授权边界：工具、有效期、模型与会话四道都算数。
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
        r#"{"mode":"single","agents":[{"ref":"a","objective":"做事"}],"request_id":"r1"}"#,
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
        r#"{"mode":"single","agents":[{"name":"x","modules":["m1"],"model":"other","objective":"做事"}],"request_id":"r2"}"#,
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

/// create_session：先与登记处事实核对，再建立；失败不留半成品，重放不重复创建。
#[test]
pub(crate) fn create_session_validates_then_creates_once() {
    let (host, mut tools) = rig();
    let c = ctx(Some(full_grant()));
    let args = r#"{"mode":"single","agents":[{"name":"x","modules":["m1"],"model":"gpt","objective":"做事"}],"workspace":"w1","request_id":"r1"}"#;
    let out = tools.call(&c, d::CREATE, args);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("work-r1"), "{}", out.output);
    let made = host.created();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].workspace.as_deref(), Some("w1"));
    assert_eq!(made[0].agents[0].name, "x");
    assert!(made[0].agents[0].transient, "新组装的 agent 是临时项");

    let again = tools.call(&c, d::CREATE, args);
    assert_eq!(again, out, "重放回同一结果");
    assert_eq!(host.created().len(), 1, "重放不得重复创建");

    for (bad, why) in [
        (
            r#"{"mode":"single","agents":[{"name":"x","modules":["nope"],"objective":"做事"}],"request_id":"b1"}"#,
            "无此模块",
        ),
        (
            r#"{"mode":"multi","agents":[{"name":"x","modules":["m1"],"objective":"一"},{"name":"y","modules":["m1"],"objective":"二"}],"request_id":"b2"}"#,
            "同一模块只能属于一个 agent",
        ),
        (
            r#"{"mode":"multi","agents":[{"name":"x","modules":["m1"],"objective":"一"}],"request_id":"b3"}"#,
            "至少要两个 agent",
        ),
        (
            r#"{"mode":"single","agents":[{"name":"x","modules":["m1"],"objective":"一"},{"name":"y","modules":["m2"],"objective":"二"}],"request_id":"b4"}"#,
            "只接受一个 agent",
        ),
        (
            r#"{"mode":"single","agents":[{"ref":"nope","objective":"做事"}],"request_id":"b5"}"#,
            "不在登记处",
        ),
        (
            r#"{"mode":"single","agents":[{"name":"a/b","modules":["m1"],"objective":"做事"}],"request_id":"b6"}"#,
            "名字不合法",
        ),
        (
            r#"{"mode":"single","agents":[{"name":"x","modules":["m1"],"objective":"做事","extra":1}],"request_id":"b7"}"#,
            "不认识的键",
        ),
        (
            r#"{"mode":"single","agents":[{"name":"x","modules":["m1"]}],"request_id":"b8"}"#,
            "objective",
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
        r#"{"mode":"single","agents":[{"ref":"a","objective":"做事"}],"request_id":"r9"}"#,
    );
    assert!(!out.ok && out.output.contains("磁盘满"), "{}", out.output);
    assert_eq!(host.created().len(), 1);
    host.clear_fail_create();
    let out = tools.call(
        &c,
        d::CREATE,
        r#"{"mode":"single","agents":[{"ref":"a","objective":"做事"}],"request_id":"r9"}"#,
    );
    assert!(out.ok, "失败不记账，重放要再试：{}", out.output);
    assert_eq!(host.created().len(), 2);
}

/// send_session_message：来源如实标记、多目标部分成功、代答必须带独立引用。
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
    assert!(
        out.output.contains("core_proxy"),
        "来源必须如实标记：{}",
        out.output
    );

    let calls = host.calls().len();
    let again = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["t1","t2"],"message":"继续","kind":"task","request_id":"r1"}"#,
    );
    assert_eq!(again, out, "重放回同一结果（含部分失败）");
    assert_eq!(host.calls().len(), calls, "重放不再动宿主");

    let out = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["t1"],"message":"好","kind":"user_reply","request_id":"r2"}"#,
    );
    assert!(
        !out.ok && out.output.contains("source_ref"),
        "{}",
        out.output
    );

    let out = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["t1"],"message":"好","kind":"user_reply","source_ref":"用户第 3 句","request_id":"r3"}"#,
    );
    assert!(out.ok, "{}", out.output);
    let last = host.relayed().pop().expect("有转达记录");
    assert_eq!(last.source, d::Source::CoreProxy);
    assert_eq!(last.source_ref.as_deref(), Some("用户第 3 句"));
}

/// observe_session：只读、不回消息正文、不推进；非法 view 不碰宿主。
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

/// read_session_messages：0 = 最新一条，按新→旧，可继续往更早翻。
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

/// control_session：回执是实际状态、重放幂等、宿主拒绝照实回报。
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

    host.fail_control("close");
    let bad = tools.call(
        &c,
        d::CONTROL,
        r#"{"session_id":"s1","action":"close","reason":"收尾","request_id":"r2"}"#,
    );
    assert!(
        !bad.ok && bad.output.contains("不能 close"),
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

/// 声明层先挡形状（没写的键、类型不对），语义层再挡取值；都不是代理工具的直接拒绝。
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
/// 真实宿主（队列桥）：catalog 读登记处、observe 不回正文、messages 倒查到真实转录；
/// 目标不存在 / 未实现的一律如实报错，不假装成功。
#[test]
pub(crate) fn the_real_bridge_reads_catalog_and_session_messages() {
    use crate::capabilities::conductor::ports::ProxyHost;
    use crate::capabilities::conductor::service::proxy::ProxyBridge;
    use crate::capabilities::workspace::api::ModuleManifest;

    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, ops) = super::super::ops_with(vec![module], vec![]);
    let (opened, _head) = ops
        .sessions
        .create_work(super::super::single_work("w-real", &["m1"]))
        .expect("建工作");
    let sid = opened.sid.clone();
    // 直接追一条转录行：倒查的确定性不依赖模型通道。
    let ev = serde_json::json!({
        "type": "transcript",
        "lines": [{"id": 7, "line": "第一条", "speaker": "用户", "verb": "说", "kind": "user"}]
    });
    {
        let sid = sid.clone();
        handle
            .call(move |core| core.history_append(&sid, &[ev]))
            .expect("追一条转录");
    }

    let bridge: Arc<dyn ProxyHost + Send + Sync> = Arc::new(ProxyBridge::new(handle.clone()));
    let mut tools = ProxyTools::new(bridge, test_systools().tools, test_prompts().tools());
    let c = ctx(Some(full_grant()));

    // 清单：只回公开事实（模型视图里没有密钥字段）。
    let out = tools.call(&c, d::CATALOG, r#"{"scope":"all"}"#);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("m1"), "{}", out.output);
    assert!(!out.output.contains("api_key"), "{}", out.output);

    // 观察：回元信息（含消息条数），不回正文。
    let args = format!(r#"{{"session_id":"{}","view":"status"}}"#, sid);
    let out = tools.call(&c, d::OBSERVE, &args);
    assert!(out.ok, "{}", out.output);
    let v: serde_json::Value = serde_json::from_str(&out.output).expect("JSON");
    assert!(
        v["message_count"].as_u64().unwrap_or(0) >= 1,
        "{}",
        out.output
    );
    assert!(
        !out.output.contains("第一条"),
        "观察不得回正文：{}",
        out.output
    );

    // 倒查：0 = 最新一条。
    let args = format!(r#"{{"session_id":"{}","from":0,"count":1}}"#, sid);
    let out = tools.call(&c, d::MESSAGES, &args);
    assert!(out.ok, "{}", out.output);
    let w: serde_json::Value = serde_json::from_str(&out.output).expect("JSON");
    assert_eq!(w["messages"][0]["text"], "第一条", "{}", out.output);

    // 父会话来自调用上下文（这里 = "main"，不存在）：如实拒绝，不接受模型自参。
    let out = tools.call(
        &c,
        d::CREATE,
        r#"{"mode":"single","agents":[{"name":"x","modules":["m1"],"objective":"y"}],"request_id":"z"}"#,
    );
    assert!(!out.ok && out.output.contains("无此会话"), "{}", out.output);
    // 转达：目标不存在时如实拒绝。
    let out = tools.call(
        &c,
        d::SEND,
        r#"{"targets":["s1"],"message":"好","kind":"task","request_id":"z2"}"#,
    );
    assert!(!out.ok && out.output.contains("无此会话"), "{}", out.output);
    // 级联停止已实现：即使没在跑也如实回执（不假装停了一个不存在的会话）。
    let out = tools.call(
        &c,
        d::CONTROL,
        r#"{"session_id":"s1","action":"stop","reason":"停","request_id":"z3"}"#,
    );
    assert!(out.ok && out.output.contains("stopped:0"), "{}", out.output);
}
/// 真实宿主：建**子工作**——编排归属是父会话，`own_work` 让它有自己的 work/ 与沙箱。
#[test]
pub(crate) fn the_real_bridge_creates_a_child_work() {
    use crate::capabilities::conductor::ports::ProxyHost;
    use crate::capabilities::conductor::service::proxy::ProxyBridge;
    use crate::capabilities::workspace::api::ModuleManifest;

    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, ops) = super::super::ops_with(vec![module], vec![]);
    let (parent, _head) = ops
        .sessions
        .create_work(super::super::single_work("w-parent", &["m1"]))
        .expect("建父工作");
    let parent_sid = parent.sid.clone();

    let bridge: Arc<dyn ProxyHost + Send + Sync> = Arc::new(ProxyBridge::new(handle.clone()));
    let mut tools = ProxyTools::new(bridge, test_systools().tools, test_prompts().tools());
    let c = d::ProxyCall {
        source: d::Source::CoreProxy,
        grant: Some(full_grant()),
        parent: Some(parent_sid.clone()),
        now: 1000,
    };
    let args = r#"{"mode":"single","agents":[{"name":"c1","modules":["m1"],"objective":"做事"}],"workspace":"子活","request_id":"r1"}"#;
    let out = tools.call(&c, d::CREATE, args);
    assert!(out.ok, "{}", out.output);
    let v: serde_json::Value = serde_json::from_str(&out.output).expect("JSON");
    let child = v["session"].as_str().expect("有会话引用").to_string();
    assert!(child.starts_with(&format!("{}--", parent_sid)), "{}", child);

    let (meta, _) = ops.history.open(&child).expect("能打开子工作");
    assert_eq!(meta.parent.as_deref(), Some(parent_sid.as_str()));
    assert!(meta.own_work, "子工作有自己的 work/ 与沙箱");
    assert_eq!(meta.mode, "single");

    let out2 = tools.call(&c, d::CREATE, args);
    assert_eq!(out2, out, "重放同一个 request_id 不再建第二个");

    // 转达：单 agent 子会话以“核心派的活”注入并点火（不等它跑完）。
    let args = format!(
        r#"{{"targets":["{}"],"message":"做这个","kind":"task","request_id":"s1"}}"#,
        child
    );
    let out = tools.call(&c, d::SEND, &args);
    assert!(out.ok, "{}", out.output);
}
/// 代理工具的执行者：按名字认领，执行经队列桥回核心线程——代理会话因此是普通成员会话。
#[test]
pub(crate) fn the_proxy_handler_owns_and_runs_proxy_tools() {
    use crate::capabilities::conductor::ports::ProxyHost;
    use crate::capabilities::conductor::service::proxy::{ProxyBridge, ProxyHandler};
    use crate::capabilities::workspace::api::ModuleManifest;
    use crate::kernel::ports::ToolHandler;

    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, _ops) = super::super::ops_with(vec![module], vec![]);
    let host: Arc<dyn ProxyHost + Send + Sync> = Arc::new(ProxyBridge::new(handle));
    let handler = ProxyHandler::new(
        host,
        test_systools().tools,
        test_prompts().tools(),
        ctx(Some(full_grant())),
    );

    assert!(handler.owns(d::CATALOG) && handler.owns(d::CREATE));
    assert!(!handler.owns("read"), "内置工具不归它");
    assert!(!handler.owns("say"), "讨论动词不归它");

    let out = handler.run("w", d::CATALOG, r#"{"scope":"agents"}"#);
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("agents"), "{}", out.output);
}
/// 代理会话跑一轮真实生成：模型请求 `catalog_agents` → 经 handler / 队列桥在核心线程执行 →
/// 工具行落进它自己的转录。这条钉住“代理会话就是普通成员会话 + 派发去特例”的整条链。
#[test]
pub(crate) fn the_proxy_session_runs_a_turn_through_the_generic_loop() {
    use crate::capabilities::conductor::api::Output;

    let (handle, ops) = super::super::ops_with(
        vec![],
        vec![
            r#"{"type":"tool","name":"catalog_agents","args":{"scope":"agents"}}"#,
            "好",
        ],
    );
    let sid = handle
        .call(|core| core.create_proxy("w-proxy", 7))
        .expect("建代理会话");
    ops.sessions
        .say(&sid, "开始", Output::Final)
        .expect("跑一轮");

    let (meta, events) = ops.history.open(&sid).expect("打开代理会话");
    assert_eq!(meta.mode, "proxy");
    assert!(meta.delegation.is_some(), "代理会话带全权委托");
    let text = serde_json::to_string(&events).expect("转录");
    assert!(
        text.contains("catalog_agents"),
        "工具行该落进转录：{}",
        text
    );
}
/// 代理会话能从落盘重建：`rebuild_session` 走 `mode="proxy"` 那一支，装回 core_proxy 面。
#[test]
pub(crate) fn the_proxy_session_rebuilds_from_disk() {
    let (handle, ops) = super::super::ops_with(vec![], vec!["好"]);
    let sid = handle
        .call(|core| core.create_proxy("w-rebuild", 3))
        .expect("建代理会话");
    let (meta, events) = ops.history.open(&sid).expect("落盘");
    assert_eq!(meta.mode, "proxy");
    let session = handle
        .call(move |core| core.rebuild_session(&meta, &events))
        .expect("重建代理会话");
    match session {
        crate::capabilities::conductor::service::Session::Single(s) => {
            let tools = s.tools.as_ref().expect("有工具环境");
            assert!(
                tools.allowed.iter().any(|t| t == d::CATALOG),
                "重建后仍是 core_proxy 面：{:?}",
                tools.allowed
            );
            assert!(!tools.with_modules);
        }
        _ => panic!("代理会话该重建为单会话形态"),
    }
}
/// 子会话停下 → 通知代理（只写一条通知行，**不转发子会话转录**）。
#[test]
pub(crate) fn a_finished_child_notifies_the_proxy_without_dumping_its_transcript() {
    use crate::capabilities::workspace::api::ModuleManifest;
    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, ops) = super::super::ops_with(vec![module], vec![]);
    let proxy = handle
        .call(|core| core.create_proxy("w-notify", 1))
        .expect("建代理会话");
    let child = handle
        .call(|core| {
            let spec = d::NewSession {
                mode: d::SessionMode::Single,
                agents: vec![d::NewAgent {
                    name: "c1".to_string(),
                    transient: true,
                    modules: vec!["m1".to_string()],
                    model: None,
                    objective: "做事".to_string(),
                }],
                workspace: None,
                request_id: "r1".to_string(),
                parent: Some("w-notify".to_string()),
            };
            Ok(core.proxy_create(&spec)?.0.session)
        })
        .expect("建子工作");
    let notified = handle
        .call({
            let c = child.clone();
            move |core| Ok(core.notify_proxy_of_child(&c))
        })
        .expect("通知");
    assert_eq!(notified.as_deref(), Some(proxy.as_str()));
    let (_, events) = ops.history.open(&proxy).expect("代理转录");
    let text = serde_json::to_string(&events).expect("JSON");
    assert!(text.contains("子会话"), "通知行该落进代理会话：{}", text);
    assert!(!text.contains("做事"), "不得把子会话的转录灌进来：{}", text);
}

/// 运行态是**落盘事实**：暂停后派发与唤醒一律被拒，关闭是终态，resume 解冻后接着走。
#[test]
pub(crate) fn run_state_gates_dispatch_and_survives_pause_close() {
    use crate::capabilities::conductor::ports::ProxyHost;
    use crate::capabilities::conductor::service::proxy::ProxyBridge;
    use crate::capabilities::workspace::api::ModuleManifest;

    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, ops) = super::super::ops_with(vec![module], vec![]);
    let (work, _) = ops
        .sessions
        .create_work(super::super::single_work("w-run", &["m1"]))
        .expect("建工作");
    let sid = work.sid.clone();
    let bridge: Arc<dyn ProxyHost + Send + Sync> = Arc::new(ProxyBridge::new(handle.clone()));
    let msg = d::Relayed {
        kind: d::MessageKind::Task,
        source: d::Source::CoreProxy,
        source_ref: None,
        parent: None,
        text: "做事".to_string(),
    };

    // 暂停：运行态落盘 + 回执如实；转达与"取会话去生成"两道闸都拒绝。
    let st = bridge
        .control(&sid, d::ControlAction::Pause, "先冻上")
        .expect("暂停");
    assert_eq!(st.state, "paused");
    assert_eq!(
        ops.history.open(&sid).expect("meta").0.run,
        RunState::Paused,
        "暂停是落盘事实，不是内存状态"
    );
    assert!(bridge.send(&sid, &msg).unwrap_err().contains("已暂停"));
    let taken = handle.call({
        let s = sid.clone();
        move |core| core.take_single(&s).map(|_| ())
    });
    assert!(
        taken.unwrap_err().contains("已暂停"),
        "叫醒路径也要被拦：暂停的会话不许被取去生成"
    );
    assert_eq!(
        bridge
            .observe(&sid, d::ObserveView::Status, None)
            .expect("观察")
            .state,
        "paused"
    );
    let replay = serde_json::to_string(&ops.history.open(&sid).expect("回放").1).expect("JSON");
    assert!(
        replay.contains("先冻上"),
        "控制原因要进可回放记录：{}",
        replay
    );

    // 关闭：终态（此刻没在跑，允许关）。
    let st = bridge
        .control(&sid, d::ControlAction::Close, "不做了")
        .expect("关闭");
    assert_eq!(st.state, "closed");
    assert_eq!(
        ops.history.open(&sid).expect("meta").0.run,
        RunState::Closed
    );
    assert!(bridge.send(&sid, &msg).unwrap_err().contains("已关闭"));
    assert!(bridge
        .control(&sid, d::ControlAction::Resume, "想反悔")
        .unwrap_err()
        .contains("已关闭"));

    // 恢复：解冻后派发放行（这条没在等门，接着走的是"继续"）。
    let (w2, _) = ops
        .sessions
        .create_work(super::super::single_work("w-run-2", &["m1"]))
        .expect("建工作");
    let sid2 = w2.sid.clone();
    bridge
        .control(&sid2, d::ControlAction::Pause, "先冻上")
        .expect("暂停");
    let st = bridge
        .control(&sid2, d::ControlAction::Resume, "接着做")
        .expect("恢复");
    assert_eq!(st.state, "active");
    assert_eq!(
        ops.history.open(&sid2).expect("meta").0.run,
        RunState::Active
    );
}

/// 转达的来源进目标会话的可回放记录：核心生成的内容不冒充用户原文。
#[test]
pub(crate) fn relay_records_its_source_on_the_target() {
    use crate::capabilities::conductor::ports::ProxyHost;
    use crate::capabilities::conductor::service::proxy::ProxyBridge;
    use crate::capabilities::workspace::api::ModuleManifest;

    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, ops) = super::super::ops_with(vec![module], vec![]);
    let (work, _) = ops
        .sessions
        .create_work(super::super::single_work("w-relay", &["m1"]))
        .expect("建工作");
    let sid = work.sid.clone();
    let bridge: Arc<dyn ProxyHost + Send + Sync> = Arc::new(ProxyBridge::new(handle));
    let msg = d::Relayed {
        kind: d::MessageKind::UserReply,
        source: d::Source::CoreProxy,
        source_ref: Some("用户第 3 句".to_string()),
        parent: None,
        text: "照这个做".to_string(),
    };
    bridge.send(&sid, &msg).expect("转达");
    let replay = serde_json::to_string(&ops.history.open(&sid).expect("回放").1).expect("JSON");
    assert!(replay.contains("核心代理转达"), "{}", replay);
    assert!(replay.contains("用户第 3 句"), "来源引用要落档：{}", replay);
    assert!(replay.contains("user_reply"), "{}", replay);
}

/// 真实宿主：mode=multi 建的是**协作子工作**（复用既有协作与节点会话机制，不另起一套）。
#[test]
pub(crate) fn the_real_bridge_creates_a_multi_agent_child_work() {
    use crate::capabilities::conductor::ports::ProxyHost;
    use crate::capabilities::conductor::service::proxy::ProxyBridge;
    use crate::capabilities::workspace::api::ModuleManifest;

    let module = |id: &str| Module {
        manifest: ModuleManifest {
            id: id.to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join(id),
    };
    let (handle, ops) = super::super::ops_with(vec![module("m1"), module("m2")], vec![]);
    let (parent, _) = ops
        .sessions
        .create_work(super::super::single_work("w-parent-multi", &["m1"]))
        .expect("建父工作");
    let bridge: Arc<dyn ProxyHost + Send + Sync> = Arc::new(ProxyBridge::new(handle));
    let mut tools = ProxyTools::new(bridge, test_systools().tools, test_prompts().tools());
    let c = d::ProxyCall {
        source: d::Source::CoreProxy,
        grant: Some(full_grant()),
        parent: Some(parent.sid.clone()),
        now: 1000,
    };
    let args = r#"{"mode":"multi","agents":[{"name":"c1","modules":["m1"],"objective":"一"},{"name":"c2","modules":["m2"],"objective":"二"}],"request_id":"m1"}"#;
    let out = tools.call(&c, d::CREATE, args);
    assert!(out.ok, "{}", out.output);
    let v: serde_json::Value = serde_json::from_str(&out.output).expect("JSON");
    let child = v["session"].as_str().expect("有会话引用").to_string();
    let agents: Vec<&str> = v["agents"]
        .as_array()
        .expect("有名单")
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    assert_eq!(agents, vec!["c1", "c2"], "{}", out.output);
    let (meta, _) = ops.history.open(&child).expect("能打开子工作");
    assert_eq!(meta.mode, "collab", "multi 建的是协作工作");
    assert_eq!(meta.parent.as_deref(), Some(parent.sid.as_str()));
    assert!(meta.own_work, "子工作有自己的 work/ 与沙箱");
    assert!(
        meta.task.is_some(),
        "协作必须有本次需求（各 agent 的 objective 合成）"
    );
    // 同一模块不得同时属于两个 agent（登记处事实核对在调用前完成，不留半成品）。
    let dup = r#"{"mode":"multi","agents":[{"name":"c1","modules":["m1"],"objective":"一"},{"name":"c2","modules":["m1"],"objective":"二"}],"request_id":"m2"}"#;
    let out = tools.call(&c, d::CREATE, dup);
    assert!(!out.ok && out.output.contains("同一模块"), "{}", out.output);
}

/// 代理会话的身份是**角色提示词**（core_proxy）+ 环境 + 调用约定，不是普通 agent 身份；
/// 反过来，普通 agent 会话也不会沾上代理角色的话。
#[test]
pub(crate) fn the_proxy_identity_is_the_role_prompt() {
    use crate::capabilities::workspace::api::ModuleManifest;

    let module = Module {
        manifest: ModuleManifest {
            id: "m1".to_string(),
            brief: "测试模块".to_string(),
            system: "你负责测试。".to_string(),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: PathBuf::from("modules").join("m1"),
    };
    let (handle, ops) = super::super::ops_with(vec![module], vec![]);
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
        "角色提示词没进去：{}",
        identity
    );
    assert!(
        identity.contains("catalog_agents"),
        "角色提示词要说明它自己的工具：{}",
        identity
    );
    assert!(identity.contains("w-role"), "环境块不能缺席：{}", identity);
    assert!(
        identity.contains("工具调用约定："),
        "调用约定不能缺席（少了模型不知道能调工具）：{}",
        identity
    );

    // 普通 agent 会话：环境与调用约定同一份口径，但不含代理角色的话。
    let (work, _) = ops
        .sessions
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
        "普通会话不该有代理角色：{}",
        plain
    );
}

/// 用户入口的第三人形态：`create_work(WorkMode::Proxy)` 建出**代理会话**——
/// 没有名单、带全权委托、运行态正常；名单 / 需求与形态不符时整条拒绝、不留半成品。
#[test]
pub(crate) fn create_work_with_proxy_mode_makes_a_delegated_session() {
    let (handle, ops) = super::super::ops_with(vec![], vec![]);
    let spec = crate::capabilities::conductor::api::WorkSpec {
        name: "w-user-proxy".to_string(),
        mode: WorkMode::Proxy,
        agents: Vec::new(),
        task: None,
        delegate: false,
        tier: Tier::Host,
    };
    let (opened, _head) = ops.sessions.create_work(spec.clone()).expect("建代理会话");
    assert!(opened.agents.is_empty(), "代理会话没有名单");
    let (meta, _) = ops.history.open(&opened.sid).expect("落盘");
    assert_eq!(meta.mode, "proxy");
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
    let views = ops
        .sessions
        .session_views(&ops.history.list().expect("列表"))
        .expect("视图");
    let v = views
        .iter()
        .find(|v| v.sid == opened.sid)
        .expect("代理会话在列表里");
    assert_eq!(v.mode, "proxy");
    assert_eq!(v.run, "active");
    assert!(!v.can_update_task, "代理会话没有本次需求");

    // 与形态不符的载荷：整条拒绝，不留半成品。
    let mut bad = spec.clone();
    bad.name = "w-user-proxy-2".to_string();
    bad.agents = vec![crate::capabilities::conductor::api::AgentInstance {
        name: "a".to_string(),
        transient: true,
        modules: vec!["m1".to_string()],
        model: None,
    }];
    assert!(ops
        .sessions
        .create_work(bad)
        .unwrap_err()
        .contains("不接受 agent 名单"));
    assert!(
        ops.history.open("w-user-proxy-2").is_err(),
        "拒绝后不留半成品"
    );

    // 档位与其余形态同一把尺子：用户选虚拟机档就按它建（承载不了则如实拒绝、什么都不留）。
    let mut vm = spec;
    vm.name = "w-user-proxy-vm".to_string();
    vm.tier = Tier::Vm;
    match ops.sessions.create_work(vm) {
        Ok((o, _)) => {
            let (m, _) = ops.history.open(&o.sid).expect("落盘");
            assert_eq!(m.exec.tier, Tier::Vm, "用户选的档位要落盘（子工作继承它）");
        }
        Err(e) => {
            assert!(!e.trim().is_empty(), "拒绝要把原因说清");
            assert!(
                ops.history.open("w-user-proxy-vm").is_err(),
                "拒绝后不留半成品"
            );
        }
    }
}
