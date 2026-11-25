//! 目的：测试替身流程——一个与协作无关的最小流程，用来钉住「驱动只认流程面」。
//! 管：StandIn 的流程面实现（开场推一步、问一个人、吸收、收工、处置自己的关卡）与它自己的事件。
//! 不管：真实流程（在 collab）；本文件不碰 turn 的会话表、落盘与呈现。
//! 联动：src/tests/flow.rs 把它塞进 `Session::Flow`，走 `Turn::collab_advance` 的同一条驱动。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::units::agentloop::api::MemberTurn;
use crate::units::llm::api::{CompleteOpts, Msg};
use crate::units::tools::api::Tools;
use crate::units::transcript::api::{Gate, SessionEvent};
use crate::units::turn_state::api::{Flow, GateTicket, Handback, MemberAsk, Settle};

/// 目的：最小流程：开场推一条自己的事件，问名单里第一个成员一回合，吸收后收工。
/// 约束：只用流程面（不认协作的任何字段）；事件走 `SessionEvent::Push`，呈现侧不必认识它。
pub(crate) struct StandIn {
    /// 目的：本回合的工具面（驱动拿去解析身份槽；替身直接沿用装配时的那一份）。
    systools: Arc<dyn Tools>,
    /// 目的：已经让出过那一步没有（让出一次就够，用来走完 take_ask → handback → feed）。
    asked: bool,
    /// 目的：成员回合已经吸收没有。
    fed: bool,
    /// 目的：流程自己的收工标记（驱动只问 `is_done`）。
    done: bool,
    /// 目的：自己的行号（回档时由 lifecycle 设置）。
    next_line: u64,
    /// 目的：停止位（替身不轮询，只按契约收下）。
    cancel: Option<Arc<AtomicBool>>,
    /// 目的：最近一次成员回合失败的原文（替身只记下，不另作反应）。
    failed: Option<String>,
    /// 目的：自己的那一关被处置过没有（用例的断言口）。
    settled: bool,
}

impl StandIn {
    /// 目的：造一个替身流程；工具面由调用方给（与真实流程同一条路）。
    pub(crate) fn new(systools: Arc<dyn Tools>) -> StandIn {
        StandIn {
            systools,
            asked: false,
            fed: false,
            done: false,
            next_line: 0,
            cancel: None,
            failed: None,
            settled: false,
        }
    }

    /// 目的：这一趟吸收过成员回合没有（用例的断言口）。
    pub(crate) fn has_fed(&self) -> bool {
        self.fed
    }

    /// 目的：自己的那一关被处置过没有（用例的断言口）。
    pub(crate) fn has_settled(&self) -> bool {
        self.settled
    }
}

impl Flow for StandIn {
    fn set_cancel(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = Some(cancel);
    }

    fn start(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        sink(SessionEvent::push(
            "standin",
            serde_json::json!({ "step": "open" }),
        ));
    }

    fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.start(sink);
    }

    fn step(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        // 吸收过之后就是收工这一步：如实说一声，再由 `is_done` 让驱动停下。
        sink(SessionEvent::push(
            "standin",
            serde_json::json!({ "step": "close" }),
        ));
    }

    fn take_ask(&mut self) -> Option<MemberAsk> {
        if self.asked || self.fed {
            return None;
        }
        self.asked = true;
        Some(MemberAsk {
            index: 0,
            member: "a".to_string(),
            seat: "solo".to_string(),
            identity: "你是替身流程问出来的一步：说一句就行。".to_string(),
            turn: Vec::<Msg>::new(),
            turn_id: 1,
            round: 1,
            opts: CompleteOpts::plain(false),
        })
    }

    fn handback(&mut self, _ask: &MemberAsk, _turn: &MemberTurn) -> Handback {
        Handback::Fed
    }

    fn feed(&mut self, _ask: &MemberAsk, _turn: MemberTurn, sink: &mut dyn FnMut(SessionEvent)) {
        self.fed = true;
        self.done = true;
        sink(SessionEvent::push(
            "standin",
            serde_json::json!({ "step": "fed" }),
        ));
    }

    fn pass_over(&mut self, _ask: &MemberAsk, _sink: &mut dyn FnMut(SessionEvent)) {}

    fn failed(&mut self, err: &str) {
        self.failed = Some(err.to_string());
    }

    fn next_line(&self) -> u64 {
        self.next_line
    }

    fn set_next_line(&mut self, next: u64) {
        self.next_line = next;
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn settles_inline(&self, gate: &Gate) -> bool {
        gate.kind == "standin"
    }

    fn settle(
        &mut self,
        ticket: &GateTicket,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<Settle, String> {
        // 替身的关卡只有一种处置：记下答案、推一条自己的事件、放行（不等异步步骤）。
        self.settled = true;
        sink(SessionEvent::push(
            "standin",
            serde_json::json!({ "step": "settled", "option": ticket.option }),
        ));
        Ok(Settle {
            resume: false,
            roster: None,
        })
    }
}
