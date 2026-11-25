//! 目的：动作目录与分发——模块工具、登记处动作、模型保存的人机边界。
//! 管：调用者校验、目录可见性、两种结果形状、原生与信封模型保存。
//! 不管：会话动作的字段（`session.rs`）、guest 诊断（`guest.rs`）。
//! 联动：`src/units/runtime/api/action.rs` 与 `src/units/registry/api`。

use super::super::doubles::InMemoryHistory;
use super::super::doubles::{decl, model_draft, module_of, module_with_runtimes};
use super::super::{ops_with, single_work};
use super::support::spawn;
use crate::presentation::faces::Faces;
use crate::units::runtime::api::{Acted, ActionCall, Caller, SessionEvent};
use crate::units::runtime::api::{Handle, Output};
use crate::units::session_model::api::MEETING;
use std::sync::Arc;

#[test]
fn native_model_save_is_gated_by_measurement_but_envelope_is_not() {
    let handle = spawn(vec![module_of("a")], Vec::new());
    let faces = Faces::from_handle(&handle);
    let draft = |id: &str, tools| crate::units::pure::registry_model::api::ModelDraft {
        tools,
        ..model_draft(id, id, id, "p", "", 0)
    };
    // 信封：不测试，直接落盘。
    faces
        .registry
        .upsert_model(draft(
            "env-model",
            crate::units::pure::registry_model::api::ToolMode::Envelope,
        ))
        .expect("信封形态不需要实测");
    assert!(
        faces
            .registry
            .models()
            .expect("读模型")
            .iter()
            .any(|m| m.id == "env-model")
    );
    // 原生：替身没有真实供应商，实测不能给出明确结论 → 不保存。
    let err = faces
        .registry
        .upsert_model(draft(
            "native-model",
            crate::units::pure::registry_model::api::ToolMode::Native,
        ))
        .unwrap_err();
    assert!(!err.is_empty(), "实测没结论要如实报错");
    assert!(
        !faces
            .registry
            .models()
            .expect("读模型")
            .iter()
            .any(|m| m.id == "native-model"),
        "实测没结论时模型不得落盘"
    );

    // 通过替身：工具调用 + 回放形状都明确通过 → 原生模型落盘。
    let core = crate::tests::builders::native_core(
        crate::tests::builders::NativeGateway {
            scripts: std::sync::Mutex::new(Vec::new()),
            seen: Default::default(),
        },
        Arc::new(InMemoryHistory::new()),
        Arc::new(crate::tests::doubles::InMemorySysIo::new()),
    );
    let ok_handle = Handle::spawn(core).expect("起核心");
    let ok_ops = Faces::from_handle(&ok_handle);
    ok_ops
        .registry
        .upsert_model(draft(
            "native-ok",
            crate::units::pure::registry_model::api::ToolMode::Native,
        ))
        .expect("实测明确通过应允许保存");
    assert_eq!(
        ok_ops
            .registry
            .models()
            .expect("读模型")
            .iter()
            .find(|m| m.id == "native-ok")
            .map(|m| m.tools),
        Some(crate::units::pure::registry_model::api::ToolMode::Native),
        "保存的就是实测通过的原生形态"
    );
}

#[test]
fn suggest_models_pushes_on_a_system_session_and_leaves_no_trace() {
    let handle = spawn(
        vec![module_of("a")],
        vec![
            "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}",
        ],
    );
    let faces = Faces::from_handle(&handle);
    let agents = faces
        .work
        .suggest_models("做个东西", MEETING)
        .expect("核心推荐");
    assert_eq!(agents.len(), 1, "回包只给名单（渲染那一步的契约不变）");
    let (batches, _head, _oldest) = handle.events().snapshot(
        Some(crate::units::session_model::api::SYSTEM_SID_SUGGEST),
        0,
    );
    assert!(
        !batches.is_empty(),
        "核心这一趟的行必须推出来（系统会话也是推的落脚点）"
    );
    let _: Vec<&SessionEvent> = batches.iter().flat_map(|l| l.events.iter()).collect();
    assert!(
        faces.history.list().expect("历史").is_empty(),
        "系统会话只推不留：推荐没有工作区，不该落盘"
    );
    assert!(
        !faces
            .work
            .session_views(&[])
            .expect("会话视图")
            .iter()
            .any(|v| v.sid == crate::units::session_model::api::SYSTEM_SID_SUGGEST),
        "系统会话不进会话列表（前端因此不会为它建标签页）"
    );
}

#[test]
fn pick_agents_names_the_unknown_and_lists_the_rest() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    faces
        .registry
        .upsert_agent("甲", &["a".to_string()], "", "", "")
        .expect("建 agent");
    let picked = faces
        .registry
        .pick_agents(&["甲".to_string()])
        .expect("点名");
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].name, "甲");
    let err = faces.registry.pick_agents(&["乙".to_string()]).unwrap_err();
    assert!(
        err.contains("无此 agent：乙") && err.contains("甲"),
        "{err}"
    );
    assert!(faces.registry.pick_agents(&[]).is_err(), "没点名 = 报错");
    let views = faces.registry.agents().expect("读登记处");
    assert_eq!(
        views
            .iter()
            .map(crate::units::pure::registry_model::api::agent_meta)
            .count(),
        1,
        "视图 → 实例不丢项"
    );
}

#[test]
fn act_dispatches_to_the_two_result_shapes() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    let sid = faces
        .work
        .create_work(single_work("w", &["a"]))
        .expect("建会话")
        .0
        .sid;

    match faces
        .actions
        .act(ActionCall {
            id: "send_message".to_string(),
            args: serde_json::json!({ "session_id": sid, "text": "你好" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("说一句")
    {
        Acted::Advanced(adv) => assert!(adv.head > 0, "生成类只回事件台头部序号"),
        _ => panic!("说一句不该给重放或结构化结果"),
    }
    match faces
        .actions
        .act(ActionCall {
            id: "rewind".to_string(),
            args: serde_json::json!({ "session_id": sid, "mode": "archive", "id": 0 }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("回档")
    {
        Acted::Replayed(events) => {
            assert!(events.iter().all(|e| e.is_object()), "重放是线格式事件数组")
        }
        _ => panic!("回档不该给事件批或结构化结果"),
    }
    assert!(
        faces
            .actions
            .act(ActionCall {
                id: "send_message".to_string(),
                args: serde_json::json!({ "session_id": "没这个会话", "text": "x" }),
                caller: Caller::User,
                out: Output::Final,
            })
            .is_err(),
        "无此会话如实报错"
    );
}

#[test]
fn module_tools_are_actions_users_can_run_directly() {
    use super::super::builders::RecordingRunner;
    let mut m = module_of("toolbox");
    m.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let runner = Arc::new(RecordingRunner::new("工具输出", true));
    let handle = Handle::spawn(super::super::doubles::core_with_runner(
        vec![m],
        super::super::doubles::gw(std::collections::BTreeMap::new(), Vec::new()),
        runner,
    ))
    .expect("起核心线程");
    let faces = Faces::from_handle(&handle);

    // 目录里列出模块工具动作（人可直接跑）；参数契约照抄 module.yaml，外加一个可选 workspace。
    let catalog = faces
        .actions
        .catalog(&Caller::User, None)
        .expect("动作目录");
    let entry = catalog
        .iter()
        .find(|a| a.id == "module.toolbox.read_txt")
        .expect("模块工具该进动作目录");
    assert!(
        entry.params.iter().any(|p| p.name == "workspace"),
        "{:?}",
        entry.params.iter().map(|p| &p.name).collect::<Vec<_>>()
    );

    // 直接跑一次（无会话）：给一份工作目录。
    match faces
        .actions
        .act(ActionCall {
            id: "module.toolbox.read_txt".to_string(),
            args: serde_json::json!({ "workspace": "work" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("直接跑模块工具")
    {
        Acted::Done(v) => assert_eq!(v.get("ok").and_then(|b| b.as_bool()), Some(true)),
        _ => panic!("模块工具该给结构化结果"),
    }

    // 席位不能从这个入口调：会话里的模块工具走成员循环（那条路由席位面授权）。
    let err = faces
        .actions
        .act(ActionCall {
            id: "module.toolbox.read_txt".to_string(),
            args: serde_json::json!({}),
            caller: Caller::Role {
                role: "executor".to_string(),
                work: "w".to_string(),
                agent: "a".to_string(),
            },
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("用户直接调用"), "{}", err);

    // 清单里没有的模块工具如实拒绝。
    assert!(
        faces
            .actions
            .act(ActionCall {
                id: "module.nope.x".to_string(),
                args: serde_json::json!({}),
                caller: Caller::User,
                out: Output::Final,
            })
            .is_err()
    );
}

#[test]
fn registry_actions_are_user_only_and_go_through_the_dispatcher() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    // 人可以用：登记一个 agent，再从登记处读回来（走的是动作分发，不是传输层直调）。
    faces
        .actions
        .act(ActionCall {
            id: "upsert_agent".to_string(),
            args: serde_json::json!({ "name": "甲", "modules": ["a"], "chara": "登记处人设" }),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("登记 agent");
    let agents = faces.registry.agents().expect("读 agent");
    assert!(agents.iter().any(|a| a.name == "甲"), "{:?}", agents);
    assert_eq!(
        agents.iter().find(|a| a.name == "甲").unwrap().chara,
        "登记处人设",
        "动作面把 chara 交给登记处"
    );

    // 席位不能调登记处动作：供应商 / 密钥类是产品级资源，默认只给人。
    let role = Caller::Role {
        role: "core_proxy".to_string(),
        work: "w".to_string(),
        agent: "a".to_string(),
    };
    let err = faces
        .actions
        .act(ActionCall {
            id: "upsert_provider".to_string(),
            args: serde_json::json!({ "id": "p", "base_url": "u", "api_key": "k" }),
            caller: role.clone(),
            out: Output::Final,
        })
        .unwrap_err();
    assert!(err.contains("不接受这个调用者"), "{}", err);
    // 目录里对席位也不列出它们。
    let cat = faces.actions.catalog(&role, None).expect("动作目录");
    assert!(
        !cat.iter()
            .any(|a| a.id.starts_with("upsert_") || a.id == "set_settings")
    );
}

#[test]
fn module_action_on_host_tier_runs_even_without_runtime_packages() {
    let mut m = module_with_runtimes("harvest", &["python"]);
    m.manifest
        .tools
        .insert("scan".to_string(), decl("python tools/scan.py"));
    let runner = Arc::new(crate::tests::builders::RecordingRunner::new("跑了", true));
    let gateway = crate::tests::doubles::gw(std::collections::BTreeMap::new(), Vec::new());
    let handle = Handle::spawn(crate::tests::doubles::core_with_runner(
        vec![m],
        gateway,
        Arc::clone(&runner),
    ))
    .expect("起核心线程");
    let faces = Faces::from_handle(&handle);

    let cat = faces.actions.catalog(&Caller::User, None).expect("目录");
    let e = cat
        .iter()
        .find(|a| a.id == "module.harvest.scan")
        .expect("模块工具进目录");
    assert!(
        e.available,
        "本机档按宿主 PATH / 运行包满足能力（reason={}）",
        e.reason
    );

    faces
        .actions
        .act(ActionCall {
            id: "module.harvest.scan".to_string(),
            args: serde_json::json!({}),
            caller: Caller::User,
            out: Output::Final,
        })
        .expect("本机档直跑要放行");
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        1,
        "真的执行了一次（不是静默跳过）"
    );
}

#[test]
fn action_table_rejects_unknown_id_value_and_caller() {
    let (_h, faces) = ops_with(vec![module_of("a")], Vec::new());
    let call = |id: &str, args: serde_json::Value, caller: Caller| ActionCall {
        id: id.to_string(),
        args,
        caller,
        out: Output::Final,
    };
    // 未知动作 id：声明表里没有 ⇒ 不落到任何实现。
    let err = faces
        .actions
        .act(call("乱来", serde_json::json!({}), Caller::User))
        .unwrap_err();
    assert!(err.contains("未知动作"), "{}", err);
    // 未知形态：参数声明过关，语义层如实拒绝。
    let err = faces
        .actions
        .act(call(
            "create_session",
            serde_json::json!({ "kind": "三个和尚", "agents": [] }),
            Caller::User,
        ))
        .unwrap_err();
    assert!(err.contains("会话类型表"), "{}", err);
    // 授权：动作表说 user 不能调只给核心代理的动作。
    let err = faces
        .actions
        .act(call(
            "catalog_agents",
            serde_json::json!({ "scope": "all" }),
            Caller::User,
        ))
        .unwrap_err();
    assert!(err.contains("不接受这个调用者"), "{}", err);
}
