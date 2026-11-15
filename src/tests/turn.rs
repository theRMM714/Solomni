//! 目的：钉 turn 能力的状态容器（TurnState）自身的契约：运行态、占名与 shell 槽互不串。
//! 管：运行标记按会话 id 收放、占名与在世表相互独立、shell 槽按会话共享且未用过时为空。
//! 不管：conductor 怎么调这些访问器（conductor 的用例在 conductor/）、会话对象本身怎么建（session）。
//! 联动：src/capabilities/turn/api.rs、docs/turn/unit-map.md。

use crate::capabilities::turn::api::TurnState;

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
