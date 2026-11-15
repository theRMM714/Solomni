//! 目的：钉 turn 能力自己的契约：状态容器（TurnState）互不串，装配纯函数按权限层过滤、只暴露就绪服务。
//! 管：运行标记按会话 id 收放、占名与在世表相互独立、shell 槽按会话共享且未用过时为空；
//!     face_tools 的发放口径、service_ops_of 的选取口径。
//! 不管：conductor 怎么调这些访问器（conductor 的用例在 conductor/）、会话对象本身怎么建（session）、
//!     装配成 MemberTools / AgentSession 的完整行为（端口齐全，归 conductor 的用例）。
//! 联动：src/capabilities/turn/api.rs、docs/turn/unit-map.md。

use crate::capabilities::permission::api::{Decision, PermissionSet};
use crate::capabilities::residents::api::{Operation, ServiceState, ServiceView};
use crate::capabilities::turn::api::{TurnState, face_tools, service_ops_of};

#[test]
fn running_marks_are_scoped_to_session_ids() {
    let mut state = TurnState::new();
    assert!(!state.is_running("甲"), "没标记过就不是运行中");
    assert!(!state.any_running(&["甲".to_string()]));
    state.mark_running("甲");
    state.mark_running("乙");
    assert!(state.is_running("甲"));
    assert!(state.is_running("乙"));
    assert!(!state.is_running("丙"), "运行标记不按名前缀或子树推断");
    let marked: Vec<String> = state.running_iter().cloned().collect();
    assert_eq!(marked.len(), 2, "两条标记各算一条，重复标记不叠算");
    state.mark_running("甲");
    assert_eq!(state.running_iter().count(), 2);
    assert!(state.any_running(&["丙".to_string(), "乙".to_string()]));
    state.unmark_running("甲");
    assert!(!state.is_running("甲"));
    assert!(!state.any_running(&["甲".to_string()]));
    assert!(state.any_running(&["乙".to_string()]));
    state.unmark_running("甲");
    assert!(state.is_running("乙"), "撤一条没标记过的不动别的会话");
}

#[test]
fn create_reservations_are_independent_of_the_live_table() {
    let mut state = TurnState::new();
    state.mark_starting("工作");
    assert!(state.is_starting("工作"));
    assert!(
        !state.holds("工作"),
        "占名只是准备段的名额，不代表会话已经在世"
    );
    state.unmark_starting("工作");
    assert!(!state.is_starting("工作"));
    state.mark_starting("甲");
    state.mark_starting("乙");
    state.unmark_starting("甲");
    assert!(state.is_starting("乙"), "撤一个占名不动另一个");
    assert!(!state.is_starting("甲"));
}

#[test]
fn shell_slot_is_shared_per_session_and_absent_before_use() {
    let state = TurnState::new();
    assert!(
        state.shell_view("甲").is_none(),
        "没跑过 shell 的会话没有快照"
    );
    let slot = state.shell_slot("甲");
    slot.start("echo hi", "host", "/");
    let again = state.shell_slot("甲");
    assert!(
        std::sync::Arc::ptr_eq(&slot, &again),
        "同一会话两次取到的是同一个槽"
    );
    let other = state.shell_slot("乙");
    assert!(!std::sync::Arc::ptr_eq(&slot, &other), "不同会话各用各的槽");
    let view = state.shell_view("甲").expect("跑过一次就有快照");
    assert_eq!(view.command, "echo hi");
    assert_eq!(view.scope, "host");
    assert!(state.shell_view("乙").is_none(), "别的会话不共享这份快照");
}

/// 目的：工具面按权限层过滤——逐工具被拒的不出现，其余保持原顺序；总表里没有的名字照发（权限层只看得出 deny）。
#[test]
fn face_tools_drops_tools_the_permission_layer_denies() {
    let book = crate::tests::prelude::test_systools().tools;
    let permissions = PermissionSet::default();
    let face = vec!["read".to_string(), "总表里没有的工具".to_string()];
    assert_eq!(
        face_tools(face.clone(), &book, &permissions),
        face,
        "默认档位下读工具可用；总表里没有的名字按发放处理"
    );
    let mut denied = permissions.clone();
    denied.tools.insert("read".to_string(), Decision::Deny);
    assert_eq!(
        face_tools(face, &book, &denied),
        vec!["总表里没有的工具".to_string()],
        "被权限层拒掉的工具不出现，剩下的保持原顺序"
    );
}

/// 目的：服务工具面只收**已就绪**的服务；按模块分组（键 = 模块 id），工具名是服务名点操作名。
#[test]
fn service_ops_only_expose_ready_residents() {
    let op = Operation {
        name: "op".to_string(),
        description: "做一件事".to_string(),
        parameters: None,
    };
    let view = |name: &str, state: ServiceState| ServiceView {
        module: "m0".to_string(),
        name: name.to_string(),
        adapter: "fake".to_string(),
        state,
        operations: vec![op.clone()],
        reason: String::new(),
    };
    let views = vec![
        view("ready", ServiceState::Ready),
        view("stopped", ServiceState::Stopped),
        view("failed", ServiceState::Failed("起不来".to_string())),
    ];
    let started = service_ops_of(&views);
    assert_eq!(started.len(), 1, "未启动与失败的服务都不进工具面");
    assert!(
        started.contains_key("m0"),
        "键是模块 id：服务操作按模块归组"
    );
    let ops = started.get("m0").expect("就绪服务要在表里");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].tool, "ready.op", "工具名 = 服务名.操作名");
    assert_eq!(ops[0].service, "ready");
    assert_eq!(ops[0].op, "op");
}
