//! 目的：核心代理六个工具的纯逻辑 的测试：入参解析、授权判据与载荷规整。
//! 管：只覆盖本模块的公开契约与拒绝文案；夹具在本文件内。
//! 不管：执行面与幂等账（在 `src/tests/runtime/proxy.rs`）、其它能力。
//! 联动：与 `src/units/conversation/proxy/domain/proxy.rs` 同模块，`#[cfg(test)] mod tests;` 指到这里。

use super::*;
use crate::units::conversation::session::api::SINGLE;

fn facts() -> Catalog {
    Catalog {
        agents: vec![crate::units::pure::registry_model::api::AgentView {
            name: "a".to_string(),
            modules: vec!["m1".to_string()],
            model: Some("gpt".to_string()),
            note: String::new(),
            chara: "已存人设".to_string(),
        }],
        modules: vec![
            ModuleFact {
                id: "m1".to_string(),
                tools: vec!["t".to_string()],
            },
            ModuleFact {
                id: "m2".to_string(),
                tools: Vec::new(),
            },
        ],
        models: vec![ModelFact {
            id: "gpt".to_string(),
            name: "GPT".to_string(),
            tools: "native".to_string(),
        }],
    }
}

/// 身份（名字 / 模块 / 模型）与登记处事实核对：不符就整条拒绝，并把**在册候选**列回给模型；
/// 任务不在 agent 上——它是会话级的 opening。
#[test]
fn resolve_new_session_checks_against_the_catalog() {
    let c = facts();
    let ok: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"name": "a", "modules": ["m1"], "model": "gpt"}],
        "task": "做事",
        "request_id": "r1"
    }))
    .unwrap();
    let spec = resolve_new_session(&ok, &c).expect("合法声明");
    assert_eq!(spec.kind, SINGLE);
    assert_eq!(spec.agents[0].modules, vec!["m1".to_string()]);
    assert!(spec.agents[0].transient);
    assert_eq!(spec.task, "做事");
    assert_eq!(spec.agents[0].chara, "", "没写人设 = 空");

    // 临时项可以带人设；这是核心代理建子会话时唯一能传人设的地方。
    let with_chara: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"name": "带人设", "modules": ["m1"], "chara": " 严谨的整理者 "}],
        "task": "做事",
        "request_id": "r0c"
    }))
    .unwrap();
    let spec = resolve_new_session(&with_chara, &c).expect("临时项人设合法");
    assert_eq!(spec.agents[0].chara, "严谨的整理者", "人设去首尾空白");

    // modules 可以省略：零模块 agent 只用内建文件工具。
    let no_modules: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"name": "纯写作", "model": "gpt"}],
        "task": "写一篇稿",
        "request_id": "r0"
    }))
    .unwrap();
    let spec = resolve_new_session(&no_modules, &c).expect("零模块 agent 合法");
    assert!(spec.agents[0].modules.is_empty(), "缺省 modules = 空");

    let unknown_module: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"name": "a", "modules": ["nope"]}],
        "task": "做事",
        "request_id": "r2"
    }))
    .unwrap();
    let err = resolve_new_session(&unknown_module, &c).unwrap_err();
    assert!(err.contains("无此模块"), "{}", err);
    assert!(err.contains("m1"), "失败要把在册模块列回去：{}", err);

    // 模型名（display name）不是 id：如实拒绝，并列出在册 id。
    let model_name: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"name": "a", "modules": ["m1"], "model": "GPT"}],
        "task": "做事",
        "request_id": "r2b"
    }))
    .unwrap();
    let err = resolve_new_session(&model_name, &c).unwrap_err();
    assert!(err.contains("无此模型"), "{}", err);
    assert!(err.contains("gpt"), "失败要把在册模型 id 列回去：{}", err);

    let empty_task: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"name": "a", "modules": ["m1"]}],
        "task": "   ",
        "request_id": "r2c"
    }))
    .unwrap();
    assert!(
        resolve_new_session(&empty_task, &c)
            .unwrap_err()
            .contains("task")
    );

    // 身份项不接受任务字段（任务是会话级的 task）。
    let extra: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"ref": "a", "objective": "做事"}],
        "task": "做事",
        "request_id": "r2d"
    }))
    .unwrap();
    assert!(
        resolve_new_session(&extra, &c)
            .unwrap_err()
            .contains("不认识的键")
    );

    let dup: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "collab",
        "agents": [
            {"name": "a", "modules": ["m1"]},
            {"name": "b", "modules": ["m1"]}
        ],
        "task": "一起做",
        "request_id": "r3"
    }))
    .unwrap();
    let shared = resolve_new_session(&dup, &c).expect("同一模块可被多个 agent 共用");
    assert_eq!(shared.agents.len(), 2);
    assert!(
        shared
            .agents
            .iter()
            .all(|a| a.modules == vec!["m1".to_string()])
    );

    // 复用登记处的 agent：模块与模型取它自己的，核心不代拟。
    let reuse: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"ref": "a"}],
        "task": "做事",
        "request_id": "r5"
    }))
    .unwrap();
    let spec = resolve_new_session(&reuse, &c).expect("复用项");
    assert!(!spec.agents[0].transient);
    assert_eq!(spec.agents[0].model.as_deref(), Some("gpt"));
    assert_eq!(spec.agents[0].chara, "已存人设", "复用项的人设取登记处事实");

    let missing: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "single",
        "agents": [{"ref": "nope"}],
        "task": "做事",
        "request_id": "r6"
    }))
    .unwrap();
    assert!(
        resolve_new_session(&missing, &c)
            .unwrap_err()
            .contains("不在登记处")
    );
}

/// 临时拼装：省略 kind、只给流程 id 建会话；kind 与 flows 只能给一个。
#[test]
fn resolve_new_session_accepts_an_inline_assembly() {
    let c = facts();
    let inline: CreateArgs = serde_json::from_value(serde_json::json!({
        "flows": ["discuss"],
        "agents": [{"ref": "a"}],
        "task": "先讨论",
        "request_id": "r7"
    }))
    .unwrap();
    let spec = resolve_new_session(&inline, &c).expect("临时拼装合法");
    assert_eq!(
        spec.kind, "",
        "临时拼装没有预设类型：类型 id 留空，靠流程说话"
    );
    assert_eq!(spec.flows.as_ref().map(Vec::len), Some(1));
    assert_eq!(spec.flows.unwrap()[0], "discuss");

    let both: CreateArgs = serde_json::from_value(serde_json::json!({
        "kind": "collab",
        "flows": ["discuss"],
        "agents": [{"ref": "a"}],
        "task": "做事",
        "request_id": "r8"
    }))
    .unwrap();
    assert!(
        resolve_new_session(&both, &c)
            .unwrap_err()
            .contains("只能给一个"),
        "预设与临时拼装不能同时给"
    );

    let none: CreateArgs = serde_json::from_value(serde_json::json!({
        "flows": [],
        "agents": [{"ref": "a"}],
        "task": "做事",
        "request_id": "r9"
    }))
    .unwrap();
    assert!(
        resolve_new_session(&none, &c)
            .unwrap_err()
            .contains("至少要有一个流程")
    );

    let barren: CreateArgs = serde_json::from_value(serde_json::json!({
        "agents": [{"ref": "a"}],
        "task": "做事",
        "request_id": "r10"
    }))
    .unwrap();
    assert!(
        resolve_new_session(&barren, &c)
            .unwrap_err()
            .contains("kind 不能为空")
    );
}
/// 授权是硬条件：没授权 / 越范围 / 超期 / 模型或会话不在授权内都拒绝。
#[test]
fn authorization_is_checked_before_anything_else() {
    let base = ProxyCall {
        grant: None,
        parent: Some("main".to_string()),
        now: 100,
    };
    assert!(
        authorize(&base, CREATE, None, None)
            .unwrap_err()
            .contains("没有代理授权")
    );

    let call = ProxyCall {
        grant: Some(Grant {
            tools: vec![CATALOG.to_string()],
            ..Default::default()
        }),
        ..base.clone()
    };
    assert!(
        authorize(&call, CREATE, None, None)
            .unwrap_err()
            .contains("不包含这个工具")
    );
    assert!(authorize(&call, CATALOG, None, None).is_ok());

    let expired = ProxyCall {
        grant: Some(Grant {
            tools: vec![CATALOG.to_string()],
            expires_at: Some(50),
            ..Default::default()
        }),
        ..base.clone()
    };
    assert!(
        authorize(&expired, CATALOG, None, None)
            .unwrap_err()
            .contains("已过期")
    );

    let scoped = ProxyCall {
        grant: Some(Grant {
            tools: vec![CREATE.to_string()],
            models: vec!["gpt".to_string()],
            sessions: vec!["s1".to_string()],
            ..Default::default()
        }),
        ..base.clone()
    };
    assert!(
        authorize(&scoped, CREATE, Some("other"), None)
            .unwrap_err()
            .contains("不覆盖这个模型")
    );
    assert!(
        authorize(&scoped, CREATE, Some("gpt"), Some("s9"))
            .unwrap_err()
            .contains("不覆盖这个会话")
    );
    assert!(authorize(&scoped, CREATE, Some("gpt"), Some("s1")).is_ok());
}

/// 转达只按 kind 记一条"由核心代理转达"的记录。
#[test]
fn relay_only_needs_targets_and_a_kind() {
    let call = ProxyCall {
        grant: None,
        parent: Some("p".to_string()),
        now: 0,
    };
    let args: SendArgs = serde_json::from_value(serde_json::json!({
        "targets": ["t1", "t1"], "message": "好", "kind": "user_reply", "request_id": "x"
    }))
    .unwrap();
    let (targets, relayed) = relay(&args, &call).unwrap();
    assert_eq!(targets.len(), 2);
    assert_eq!(relayed.kind, MessageKind::UserReply);
    assert_eq!(relayed.text, "好");
    assert_eq!(relayed.parent.as_deref(), Some("p"));

    let bad: SendArgs = serde_json::from_value(serde_json::json!({
        "targets": [], "message": "好", "kind": "task", "request_id": "x"
    }))
    .unwrap();
    assert!(relay(&bad, &call).unwrap_err().contains("targets"));

    let bad_kind: SendArgs = serde_json::from_value(serde_json::json!({
        "targets": ["t1"], "message": "好", "kind": "nope", "request_id": "x"
    }))
    .unwrap();
    assert!(relay(&bad_kind, &call).unwrap_err().contains("kind"));
}

/// 清单回执只回请求的 scope。
#[test]
fn catalog_renders_only_the_requested_scope() {
    let c = facts();
    let only = render_catalog(CatalogScope::Agents, &c).unwrap();
    assert!(only.contains("agents"));
    assert!(!only.contains("models"));
    let all = render_catalog(CatalogScope::All, &c).unwrap();
    assert!(all.contains("modules"));
    assert!(all.contains("models"));
    assert!(all.contains("已存人设"), "清单要带 agent 人设：{}", all);
}

/// 转达要代答**只认选项 id**：属于那张卡才认，其余一律如实拒绝——不给任何"从一句话猜选项"的规则
/// （猜会替用户做主：围栏卡的第一项正是"无围栏跑一次"）。
#[test]
fn relay_reply_only_accepts_an_option_id_from_that_card() {
    use crate::units::conversation::collab::api::{OPT_ASK_REPLY, OPT_BEGIN, OPT_SLATE_CANCEL};
    use crate::units::transcript::api::OptionView;
    let o = |id: &str| OptionView {
        id: id.to_string(),
        label: id.to_string(),
    };
    let slate = vec![o(OPT_SLATE_CANCEL), o(OPT_BEGIN)];
    assert_eq!(relay_reply(&slate, OPT_BEGIN), Ok(OPT_BEGIN.to_string()));
    assert_eq!(
        relay_reply(&slate, &format!(" {} ", OPT_BEGIN)),
        Ok(OPT_BEGIN.to_string()),
        "两侧空白 trim 之后仍按选项 id 校验"
    );
    assert!(
        relay_reply(&slate, "yes").is_err(),
        "不在卡上的选项不许猜（否则等于替用户做主）"
    );
    assert!(relay_reply(&slate, "").is_err(), "空串不算代答");
    let ask = vec![o(OPT_ASK_REPLY)];
    assert_eq!(
        relay_reply(&ask, OPT_ASK_REPLY),
        Ok(OPT_ASK_REPLY.to_string())
    );
}

/// 转达与控制都要留下"谁、为什么"的可回放记录。
#[test]
fn notes_record_who_really_said_it() {
    let note = relay_note(MessageKind::UserReply);
    assert!(
        note.contains("核心代理转达") && note.contains("user_reply"),
        "{}",
        note
    );
    let note = relay_note(MessageKind::Task);
    assert!(
        note.contains("核心代理转达") && note.contains("task"),
        "{}",
        note
    );
    let ctl = control_note(ControlAction::Stop, "先停一下");
    assert!(ctl.contains("stop") && ctl.contains("先停一下"), "{}", ctl);
}

/// 消息倒查的边界：`from` 以最新为 0、`count` 有上下界、缺省是 0/10。
#[test]
fn messages_request_checks_from_and_count() {
    let ok: MessagesArgs = serde_json::from_value(serde_json::json!({
        "session_id": "s1", "from": 2, "count": 5
    }))
    .unwrap();
    assert_eq!(messages_request(&ok).unwrap(), ("s1".to_string(), 2, 5));

    let default: MessagesArgs =
        serde_json::from_value(serde_json::json!({ "session_id": "s1" })).unwrap();
    assert_eq!(
        messages_request(&default).unwrap(),
        ("s1".to_string(), 0, 10)
    );

    let bad: MessagesArgs = serde_json::from_value(serde_json::json!({
        "session_id": "s1", "from": -1
    }))
    .unwrap();
    assert!(messages_request(&bad).unwrap_err().contains("from"));

    let big: MessagesArgs = serde_json::from_value(serde_json::json!({
        "session_id": "s1", "count": 51
    }))
    .unwrap();
    assert!(messages_request(&big).unwrap_err().contains("count"));
}
