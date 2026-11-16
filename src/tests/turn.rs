//! 目的：`turn` 的回合状态用例——运行态、创建占位与 shell 槽的边界。
//! 管：在世表放取的可见性、运行态与创建占位互不干扰、shell 槽按会话共用一份。
//! 不管：会话本体与生成驱动（见 `src/tests/runtime.rs`）。
//! 联动：`src/capabilities/turn/service/state.rs`、`src/capabilities/runtime/service.rs`。

use super::prelude::*;
use crate::capabilities::turn::api::TurnState;
use crate::kernel::api::SessionId;

/// 目的：验运行态按会话名记账，追加与撤销互不影响别的会话。
#[test]
fn running_marks_are_scoped_to_session_ids() {
    let mut state = TurnState::new();
    assert!(!state.is_running("a"));
    state.mark_running("a");
    state.mark_running(SessionId::from("b".to_string()));
    assert!(state.is_running("a"));
    assert!(state.is_running("b"));
    state.unmark_running("a");
    assert!(!state.is_running("a"));
    assert!(state.is_running("b"), "摘一个会话的运行态不许连带别人");
    let ids: Vec<&SessionId> = state.running_iter().collect();
    assert_eq!(ids, vec![&SessionId::from("b".to_string())]);
}

/// 目的：验创建占位与运行态是两套集合，占名不影响运行态。
#[test]
fn create_reservations_are_independent_of_the_live_table() {
    let mut state = TurnState::new();
    state.mark_starting("w");
    assert!(state.is_starting("w"));
    assert!(!state.is_running("w"), "占名不等于在跑");
    assert!(!state.holds("w"), "占名不等于已入世");
    state.unmark_starting("w");
    assert!(!state.is_starting("w"));
}

/// 目的：验 shell 槽同一会话共用一份、没用过时视图为空。
#[test]
fn shell_slot_is_shared_per_session_and_absent_before_use() {
    let state = TurnState::new();
    assert!(state.shell_view("a").is_none(), "没跑过就没有快照");
    let first = state.shell_slot("a");
    let again = state.shell_slot("a");
    assert!(std::sync::Arc::ptr_eq(&first, &again), "同一会话共用一份槽");
    assert!(!std::sync::Arc::ptr_eq(&first, &state.shell_slot("b")));
}

/// 目的：验角色工具面按权限档位过滤——deny 的（View 下的写工具）不发放，工具总表里没声明的照旧发放。
#[test]
fn face_tools_filters_denied_tools_and_keeps_undeclared_ones() {
    use crate::capabilities::permission::api::{PermissionDefaults, ProfileId};
    use crate::capabilities::tools::api::{ToolBook, ToolSchema};
    use crate::capabilities::turn::domain::assemble::face_tools;

    let schema = |capability: &str| ToolSchema {
        desc: String::new(),
        params: None,
        parallel: false,
        capability: capability.to_string(),
        effect: None,
        reach: None,
        decision: None,
        scope: false,
        callers: Vec::new(),
    };
    let mut book = ToolBook::new();
    book.insert("read".to_string(), schema("fs-read"));
    book.insert("write".to_string(), schema("fs-write"));
    let permissions = PermissionDefaults {
        profile: ProfileId::View,
        ..Default::default()
    }
    .to_permission_set();
    let face = vec![
        "read".to_string(),
        "write".to_string(),
        "missing".to_string(),
    ];
    assert_eq!(
        face_tools(face, &book, &permissions),
        vec!["read".to_string(), "missing".to_string()],
        "写工具在 View 档位下 deny，未声明的 id 放行"
    );
}

/// 目的：验只有已就绪的常驻服务进工具面，工具名按 `<服务>.<操作>` 拼。
#[test]
fn service_ops_take_only_ready_services_and_qualify_names() {
    use crate::capabilities::residents::api::{Operation, ServiceState, ServiceView};
    use crate::capabilities::turn::domain::assemble::service_ops_of;

    let op = |name: &str| Operation {
        name: name.to_string(),
        description: format!("{} 的说明", name),
        parameters: None,
    };
    let view = |name: &str, state: ServiceState| ServiceView {
        module: "m".to_string(),
        name: name.to_string(),
        adapter: "a".to_string(),
        state,
        operations: vec![op("run")],
        reason: String::new(),
    };
    let faces = service_ops_of(&[
        view("svc", ServiceState::Ready),
        view("off", ServiceState::Stopped),
    ]);
    assert_eq!(faces.len(), 1, "没启动的服务没有可发现的操作");
    let got = &faces["m"];
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].tool, "svc.run");
    assert_eq!(got[0].service, "svc");
    assert_eq!(got[0].op, "run");
}

/// 目的：验服务操作并进模块工具表，同时拼出模块工具行与服务参数行。
#[test]
fn serviced_tools_merge_ops_into_table_and_render_lines() {
    use crate::capabilities::session::api::{ModuleTools, ServiceOp};
    use crate::capabilities::turn::domain::assemble::serviced_tools;

    let mut table = std::collections::BTreeMap::new();
    table.insert(
        "m".to_string(),
        ModuleTools {
            root: std::path::PathBuf::new(),
            commands: std::collections::BTreeMap::new(),
            books: std::collections::BTreeMap::new(),
            parallel: std::collections::BTreeSet::new(),
            services: std::collections::BTreeMap::new(),
        },
    );
    let op = ServiceOp {
        tool: "svc.run".to_string(),
        service: "svc".to_string(),
        op: "run".to_string(),
        description: "跑一次".to_string(),
        parameters: Some(serde_json::json!({
            "type": "object",
            "properties": {"q": {"type": "string", "description": "查询"}}
        })),
    };
    let started = std::collections::BTreeMap::from([("m".to_string(), vec![op])]);
    let (tool_lines, param_lines) = serviced_tools(&started, &mut table);
    assert_eq!(table["m"].services.len(), 1, "操作要并进模块工具表");
    assert_eq!(tool_lines, vec!["- m：svc.run".to_string()]);
    assert_eq!(param_lines.len(), 1);
    assert!(
        param_lines[0].starts_with("- m.svc.run\n"),
        "服务参数行以 <模块>.<服务>.<操作> 开头：{}",
        param_lines[0]
    );
}

/// 目的：验准入把会话既落盘又放进在世表——先抹掉落盘，准入后两处都要恢复。
#[test]
fn admit_puts_the_session_in_the_live_table_and_persists_identity() {
    let mut core = core_with_settings_modules(InMemorySettings::new(), vec![module_of("a")]);
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .expect("建工作")
        .sid;
    let session = core.turn.take(&sid).expect("取出会话");
    let (meta, _) = core.lifecycle.history_open(&sid).expect("读元数据");
    core.lifecycle
        .history_delete(&mut core.turn, &sid)
        .expect("先抹掉落盘");
    assert!(!core.turn.holds(&sid), "抹掉后不在世");
    core.turn.admit(&meta, session).expect("准入");
    assert!(core.turn.holds(&sid), "准入后要进在世表");
    assert!(
        core.lifecycle.history_list().iter().any(|m| m.name == sid),
        "准入要把会话身份落盘"
    );
}

/// 目的：验派发闸只看运行态——停止与关闭都拒，没落盘的名字不拦。
#[test]
fn dispatch_gate_refuses_a_stopped_subtree() {
    let mut core = core_with_settings_modules(InMemorySettings::new(), vec![module_of("a")]);
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .expect("建工作")
        .sid;
    assert!(core.turn.dispatch_gate(&sid).is_ok(), "活动会话照常派发");
    core.turn
        .set_subtree_run(&sid, RunState::Stopped)
        .expect("停止子树");
    let stopped = core.turn.dispatch_gate(&sid).expect_err("停掉的会话要拒");
    assert!(stopped.contains("已停止"), "{}", stopped);
    core.turn
        .set_subtree_run(&sid, RunState::Closed)
        .expect("关闭子树");
    let closed = core.turn.dispatch_gate(&sid).expect_err("关掉的会话要拒");
    assert!(closed.contains("已关闭"), "{}", closed);
    assert!(
        core.turn.dispatch_gate("no-such-session").is_ok(),
        "没有落盘的会话不拦"
    );
}
