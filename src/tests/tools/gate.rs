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
    assert_eq!(
        Coverage::Ranges(vec![
            LineRange::new(4, 4),
            LineRange::new(1, 2),
            LineRange::new(2, 3),
            LineRange::new(3, 3),
        ])
        .normalized(),
        Coverage::Ranges(vec![LineRange::new(1, 4)]),
        "区间要排序、去重并合并相邻/重叠"
    );
    assert_eq!(
        Coverage::Ranges(vec![LineRange::new(1, 1)])
            .union(&Coverage::Ranges(vec![LineRange::new(2, 2)])),
        Coverage::Ranges(vec![LineRange::new(1, 2)]),
        "多次分段读的事实可以并成一段"
    );
    assert_eq!(
        Coverage::Ranges(vec![LineRange::new(1, 2)]).union(&Coverage::Full),
        Coverage::Full,
        "任一方是全文，并集就是全文"
    );
}

/// 目的：读事实的派生——整份读一条，分段读每段一条；越界段、空文件与非读来源不产生。
#[test]
pub(crate) fn read_facts_cover_full_reads_and_every_segment() {
    use crate::capabilities::tools::api::{OBSERVATION_FORMAT, ObservationFact, ObservationSource};
    let fact = |coverage: Coverage, total: u32| ObservationFact {
        schema: OBSERVATION_FORMAT,
        reply: 7,
        path: "note.md".to_string(),
        worktree: "a".to_string(),
        version: "v".to_string(),
        coverage,
        total_lines: total,
        source: ObservationSource::Read,
    };

    let full = fact(Coverage::Full, 10).read_facts();
    assert_eq!(full.len(), 1, "整份读只派生一条");
    assert_eq!((full[0].from, full[0].to), (1, 10));
    assert_eq!(full[0].blob, "v", "指纹就是版本");

    let ranges = fact(
        Coverage::Ranges(vec![LineRange::new(1, 50), LineRange::new(60, 400)]),
        400,
    )
    .read_facts();
    assert_eq!(
        ranges.iter().map(|r| (r.from, r.to)).collect::<Vec<_>>(),
        vec![(1, 50), (60, 400)],
        "分段读每段派生一条，大文件照样被照看"
    );

    let clamped = fact(Coverage::Ranges(vec![LineRange::new(90, 120)]), 100).read_facts();
    assert_eq!(
        (clamped[0].from, clamped[0].to),
        (90, 100),
        "末行按总行数截断"
    );
    assert!(
        fact(Coverage::Ranges(vec![LineRange::new(101, 120)]), 100)
            .read_facts()
            .is_empty(),
        "整段都在文件外：没有内容要保护"
    );
    assert!(
        fact(Coverage::Full, 0).read_facts().is_empty(),
        "空文件不产生读事实"
    );

    let mut wrote = fact(Coverage::Full, 10);
    wrote.source = ObservationSource::Write;
    assert!(wrote.read_facts().is_empty(), "写来源不能当成读过");
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
        obs.facts()
            .iter()
            .all(|f| !f.coverage.covers_all(f.total_lines)),
        "截断行不能记全文：{:?}",
        obs.facts()
    );
}

/// 目的：读取输出被回执预算截断时，模型没看到全文，不能记成全文观察。
#[test]
pub(crate) fn read_capped_by_the_result_budget_is_not_full() {
    let io = std::sync::Arc::new(InMemorySysIo::new());
    let text: String = (1..=4_000).map(|i| format!("第 {i} 行\n")).collect();
    io.seed(&["demo", "a1", "long.txt"], &text);
    let note = s(&["demo", "a1", "long.txt"]);
    let mut obs = Observations::default();
    let out = run(
        &io,
        &mut obs,
        &FakeGate::allowing(),
        "read",
        format!("{{\"path\":\"{}\"}}", note),
    );
    assert!(out.ok, "{}", out.output);
    assert!(
        out.output.chars().count() <= 16_000,
        "读回执本身必须留在发送预算内：{}",
        out.output.chars().count()
    );
    assert!(
        obs.facts()
            .iter()
            .all(|f| !f.coverage.covers_all(f.total_lines)),
        "只读了一部分就不能记全文：{:?}",
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
        crate::capabilities::tools::api::ResultBudget::full(),
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
