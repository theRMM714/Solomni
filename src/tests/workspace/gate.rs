//! 目的：先读后改门禁的会话侧测试：拒绝审计与批量判定。
//! 管：拒绝只记路径/版本/错误码、批量返回首个失败项。
//! 不管：工作区快照与版本语义（`work.rs`）。
//! 联动：`docs/session/session-model.md`。

use super::support::*;

/// 目的：门禁拒绝时写一条可追溯的审计记录，只记版本与路径、不记文件正文。
#[test]
pub(crate) fn gate_refusal_is_audited_without_file_content() {
    use crate::units::tools::api::SessionWriteGate;
    use crate::units::tools::api::{GateFault, Need, WriteGate};

    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"one\ntwo\n", 1, 0)
        .unwrap();
    let log = crate::tests::doubles::RecordingLog::new();
    let gate = SessionWriteGate {
        workspace: ws.as_ref(),
        work: "w",
        agent: "a",
        facts: &[],
        worktree: "a",
        log: Some(&log),
    };
    let err = gate.check("base.md", Need::Observed, None);
    assert_eq!(err, Err(GateFault::NotObserved));
    assert!(
        log.has("FS_NOT_OBSERVED") && log.has("base.md"),
        "审计要能看出哪条路径因为什么被拒"
    );
    let audited = log
        .lines
        .lock()
        .expect("锁")
        .iter()
        .any(|(level, at, _)| level == "warn" && at == "tools::write_gate");
    assert!(
        audited,
        "审计要落在写入门禁的日志位置上：{:?}",
        log.lines.lock().expect("锁")
    );
    assert!(!log.has("one\ntwo"), "审计不得记录文件正文");
}

/// 目的：一批写入只取一次快照，但每条仍按同一语义判定；返回第一条失败的序号。
#[test]
pub(crate) fn batch_gate_returns_the_first_failed_item() {
    use crate::units::tools::api::SessionWriteGate;
    use crate::units::tools::api::{GateFault, GateItem, Need, WriteGate};

    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"one\ntwo\n", 1, 0)
        .unwrap();
    let gate = SessionWriteGate {
        workspace: ws.as_ref(),
        work: "w",
        agent: "a",
        facts: &[],
        worktree: "a",
        log: None,
    };
    let items = [
        GateItem {
            path: "scratch.md",
            need: Need::Full,
            current: None,
        },
        GateItem {
            path: "base.md",
            need: Need::Observed,
            current: None,
        },
    ];
    assert_eq!(
        gate.check_many(&items),
        Err((1, GateFault::NotObserved)),
        "untracked 放行，受管未读路径拒"
    );
    assert_eq!(gate.check_many(&items[..1]), Ok(()));
}
