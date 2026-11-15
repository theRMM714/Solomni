//! 目的：`turn` 的回合状态用例——运行态、创建占位与 shell 槽的边界。
//! 管：在世表放取的可见性、运行态与创建占位互不干扰、shell 槽按会话共用一份。
//! 不管：会话本体与生成驱动（见 `src/tests/conductor.rs`）。
//! 联动：`src/capabilities/turn/service/state.rs`、`src/capabilities/conductor/service.rs`。

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
