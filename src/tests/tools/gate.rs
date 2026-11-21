//! 目的：写入门禁的契约——覆盖判定、当前版本比较、拒绝码。
//! 管：Coverage 并集、write/edit 请门禁时带的 Need 与 preimage。
//! 不管：工作区如何判定 tracked（在 workspace 测试）。
//! 联动：tools::api 的 WriteGate / GateFault。

use super::super::builders::*;
use super::super::prelude::*;
use crate::capabilities::tools::api::{
    Coverage, GateFault, LineRange, Need, Observations, WriteGate,
};
use crate::capabilities::workspace::api::{GateFile, content_hash};
use std::sync::Mutex;

#[test]
pub(crate) fn coverage_union_requires_every_line() {
    assert!(Coverage::Full.covers_all(0));
    assert!(!Coverage::Ranges(vec![]).covers_all(0));
    assert!(Coverage::Ranges(vec![LineRange::new(1, 1)]).covers_all(1));
    assert!(!Coverage::Ranges(vec![LineRange::new(2, 2)]).covers_all(2));
    assert!(
        Coverage::Ranges(vec![LineRange::new(1, 2), LineRange::new(3, 4)]).covers_all(4),
        "相邻区间并集覆盖全文"
    );
    assert!(
        Coverage::Ranges(vec![LineRange::new(1, 3), LineRange::new(2, 4)]).covers_all(4),
        "重叠区间按并集算"
    );
    assert!(
        !Coverage::Ranges(vec![LineRange::new(1, 2), LineRange::new(4, 4)]).covers_all(4),
        "缺第 3 行就不算全文"
    );
}

/// 目的：单行被显示上限截断时，读到的不是原文，不能成为"全文观察"。
#[test]
pub(crate) fn read_with_a_truncated_line_is_not_full() {
    let io = std::sync::Arc::new(InMemorySysIo::new());
    io.seed(&["demo", "a1", "note.txt"], &"x".repeat(2_500));
    let note = s(&["demo", "a1", "note.txt"]);
    let mut obs = Observations::default();
    let out = run(
        &io,
        &mut obs,
        &FakeGate::allowing(),
        "read",
        format!("{{\"path\":\"{}\"}}", note),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("本行超过"), "{}", out.output);
    assert!(
        obs.facts().iter().all(|f| !f.is_full()),
        "截断行不能记全文：{:?}",
        obs.facts()
    );
}

#[derive(Default)]
struct FakeGate {
    calls: Mutex<Vec<(String, Need, Option<GateFile>)>>,
    answer: Option<GateFault>,
}

impl FakeGate {
    fn refusing(fault: GateFault) -> FakeGate {
        FakeGate {
            calls: Mutex::new(Vec::new()),
            answer: Some(fault),
        }
    }

    fn allowing() -> FakeGate {
        FakeGate::default()
    }

    fn calls(&self) -> Vec<(String, Need, Option<GateFile>)> {
        self.calls.lock().expect("锁").clone()
    }
}

impl WriteGate for FakeGate {
    fn check(&self, path: &str, need: Need, current: Option<&GateFile>) -> Result<(), GateFault> {
        self.calls
            .lock()
            .expect("锁")
            .push((path.to_string(), need, current.cloned()));
        match &self.answer {
            Some(fault) => Err(fault.clone()),
            None => Ok(()),
        }
    }
}

fn run(
    io: &std::sync::Arc<InMemorySysIo>,
    obs: &mut Observations,
    gate: &dyn WriteGate,
    tool: &str,
    args: String,
) -> crate::kernel::api::types::ToolOutcome {
    let sb = test_sandbox("a1", &[]);
    let exec = test_tools_svc_with(
        std::sync::Arc::new(SilentRunner),
        std::sync::Arc::clone(io),
        std::sync::Arc::new(NoFenceHost),
    );
    let done = exec.run_builtin(
        &sb,
        &test_systools().tools,
        std::mem::take(obs),
        Some(gate),
        tool,
        &args,
    );
    *obs = done.observations;
    done.outcome
}

#[test]
pub(crate) fn write_checks_full_coverage_against_the_current_version() {
    let io = std::sync::Arc::new(InMemorySysIo::new());
    let note = s(&["demo", "a1", "note.txt"]);
    io.seed(&["demo", "a1", "note.txt"], "旧内容");
    let gate = FakeGate::refusing(GateFault::NotObserved);
    let mut obs = Observations::default();
    let out = run(
        &io,
        &mut obs,
        &gate,
        "write",
        format!("{{\"path\":\"{}\",\"content\":\"新内容\"}}", note),
    );
    assert!(!out.ok);
    assert!(out.output.contains("FS_NOT_OBSERVED"), "{}", out.output);
    assert!(
        out.notes.iter().any(|n| n.code == "FS_NOT_OBSERVED"),
        "结构化拒绝码也要带上"
    );
    let calls = gate.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "note.txt");
    assert_eq!(calls[0].1, Need::Full);
    assert_eq!(
        calls[0].2.as_ref().map(|f| f.hash.clone()),
        Some(content_hash("旧内容".as_bytes())),
        "门禁比较的是磁盘上的整份版本"
    );
    assert_eq!(
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
        Some("旧内容"),
        "拒绝时不写盘"
    );
}

#[test]
pub(crate) fn edit_checks_observed_and_replace_all_checks_full() {
    let io = std::sync::Arc::new(InMemorySysIo::new());
    let note = s(&["demo", "a1", "note.txt"]);
    io.seed(&["demo", "a1", "note.txt"], "一\n二\n二\n");
    let gate = FakeGate::allowing();
    let mut obs = Observations::default();
    let out = run(
        &io,
        &mut obs,
        &gate,
        "edit",
        format!(
            "{{\"path\":\"{}\",\"old_string\":\"一\",\"new_string\":\"壹\"}}",
            note
        ),
    );
    assert!(out.ok, "{}", out.output);
    let calls = gate.calls();
    assert_eq!(calls[0].1, Need::Observed);

    let all = run(
        &io,
        &mut obs,
        &gate,
        "edit",
        format!(
            "{{\"path\":\"{}\",\"old_string\":\"二\",\"new_string\":\"四\",\"replace_all\":true}}",
            note
        ),
    );
    assert!(all.ok, "{}", all.output);
    let calls = gate.calls();
    assert_eq!(calls[1].1, Need::Full, "replace_all 会覆盖多处，要求全文");
    assert_eq!(
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
        Some("壹\n四\n四\n")
    );
}
