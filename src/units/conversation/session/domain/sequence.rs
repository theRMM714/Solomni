//! 目的：复合流程——装配期把若干段流程串成一条：前一段收尾就把产物交给下一段（intake），转录行号接上。
//! 管：按序推进、交接产物与行号；把流程面整套转给**当前这一段**。
//! 不管：任何一段自己的状态机（归各流程）；会话类型表与装配（归会话能力的装配与组合根）。
//! 联动：`systools/profiles.yaml` 里一条会话类型的 flows 序列由它承载；驱动只认 `Flow`。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::units::agentloop::api::MemberTurn;
use crate::units::conversation::session::api::{
    Flow, GateTicket, Handback, MemberAsk, Settle, WorkPlan,
};
use crate::units::pure::registry_model::api::AgentMeta;
use crate::units::transcript::api::{Gate, SessionEvent};
use crate::units::workspace::api::Sandboxes;

/// 目的：一串按顺序跑的流程段：前一段收尾后产物交给下一段，行号接着上一段。
pub struct Sequence {
    steps: Vec<Box<dyn Flow>>,
    at: usize,
}

impl Sequence {
    /// 目的：按给定顺序造一条复合流程（装配期只造一次；顺序即执行顺序）。
    pub fn new(steps: Vec<Box<dyn Flow>>) -> Sequence {
        Sequence { steps, at: 0 }
    }

    fn current(&self) -> Option<&dyn Flow> {
        self.steps.get(self.at).map(|s| &**s as &dyn Flow)
    }

    fn current_mut(&mut self) -> Option<&mut (dyn Flow + 'static)> {
        self.steps
            .get_mut(self.at)
            .map(|s| &mut **s as &mut dyn Flow)
    }

    /// 目的：当前这一段已收尾就往下一段交：产物给 intake，行号接着上一段（没交东西也接行号）。
    fn settle_forward(&mut self) {
        while self.at + 1 < self.steps.len() && self.steps[self.at].is_done() {
            let line = self.steps[self.at].next_line();
            let made = self.steps[self.at].output();
            self.at += 1;
            let next = &mut self.steps[self.at];
            next.set_next_line(line);
            if let Some(text) = made {
                next.intake(text);
            }
        }
    }
}

impl Flow for Sequence {
    fn set_cancel(&mut self, cancel: Arc<AtomicBool>) {
        for step in &mut self.steps {
            step.set_cancel(Arc::clone(&cancel));
        }
    }

    fn start(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.settle_forward();
        if let Some(step) = self.current_mut() {
            step.start(sink);
        }
    }

    fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.settle_forward();
        if let Some(step) = self.current_mut() {
            step.resume(sink);
        }
    }

    fn step(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.settle_forward();
        if let Some(step) = self.current_mut() {
            step.step(sink);
        }
    }

    fn take_ask(&mut self) -> Option<MemberAsk> {
        self.settle_forward();
        self.current_mut().and_then(|step| step.take_ask())
    }

    fn handback(&mut self, ask: &MemberAsk, turn: &MemberTurn) -> Handback {
        match self.current_mut() {
            Some(step) => step.handback(ask, turn),
            None => Handback::PassOver,
        }
    }

    fn feed(&mut self, ask: &MemberAsk, turn: MemberTurn, sink: &mut dyn FnMut(SessionEvent)) {
        if let Some(step) = self.current_mut() {
            step.feed(ask, turn, sink);
        }
    }

    fn pass_over(&mut self, ask: &MemberAsk, sink: &mut dyn FnMut(SessionEvent)) {
        if let Some(step) = self.current_mut() {
            step.pass_over(ask, sink);
        }
    }

    fn failed(&mut self, err: &str) {
        if let Some(step) = self.current_mut() {
            step.failed(err);
        }
    }

    fn begin_with(&mut self, task: &str, sink: &mut dyn FnMut(SessionEvent)) {
        // 需求只交给第一段：后面的段从上一段的产物接手。
        if let Some(first) = self.steps.first_mut() {
            first.begin_with(task, sink);
        }
    }

    fn next_line(&self) -> u64 {
        self.current().map(|s| s.next_line()).unwrap_or(0)
    }

    fn set_next_line(&mut self, next: u64) {
        for step in &mut self.steps {
            step.set_next_line(next);
        }
    }

    fn adopt_roster(&mut self, roster: &[AgentMeta]) {
        for step in &mut self.steps {
            step.adopt_roster(roster);
        }
    }

    fn plan(&self) -> Option<&dyn WorkPlan> {
        self.current().and_then(|s| s.plan())
    }

    fn plan_mut(&mut self) -> Option<&mut dyn WorkPlan> {
        self.current_mut().and_then(|s| s.plan_mut())
    }

    fn withdraw_agree(&mut self, agent: &str, sink: &mut dyn FnMut(SessionEvent)) -> bool {
        match self.current_mut() {
            Some(step) => step.withdraw_agree(agent, sink),
            None => false,
        }
    }

    fn adapt_text(&self, text: &str) -> String {
        match self.current() {
            Some(step) => step.adapt_text(text),
            None => text.to_string(),
        }
    }

    fn set_sandboxes(&mut self, sandboxes: Sandboxes) {
        // 沙箱是会话级事实：每一段都要认得出路径（后一段不再是同一个对象）。
        for step in &mut self.steps {
            step.set_sandboxes(sandboxes.clone());
        }
    }

    fn turn_cursor(&mut self) -> u64 {
        self.current_mut().map(|s| s.turn_cursor()).unwrap_or(0)
    }

    fn default_answer(&self) -> Option<String> {
        self.current().and_then(|s| s.default_answer())
    }

    fn is_done(&self) -> bool {
        self.steps.last().is_some_and(|s| s.is_done())
    }

    fn member_seat(&self) -> Option<&'static str> {
        self.current().and_then(|s| s.member_seat())
    }

    fn notice(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        match self.current_mut() {
            Some(step) => step.notice(text, sink),
            None => sink(SessionEvent::Notice(text.to_string())),
        }
    }

    fn settle(
        &mut self,
        ticket: &GateTicket,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<Settle, String> {
        self.settle_forward();
        let at = self.at;
        // 卡也可能来自前面那一段（回档后两段的事实交叠）：先试当前段，再按段找一次。
        if let Some(step) = self.steps.get_mut(at)
            && let Ok(settled) = step.settle(ticket, sink)
        {
            return Ok(settled);
        }
        for (i, step) in self.steps.iter_mut().enumerate() {
            if i == at {
                continue;
            }
            if let Ok(settled) = step.settle(ticket, sink) {
                return Ok(settled);
            }
        }
        Err(format!("这张卡不是本会话的流程发的：{}", ticket.gate.kind))
    }

    fn settles_inline(&self, gate: &Gate) -> bool {
        self.steps.iter().any(|s| s.settles_inline(gate))
    }

    fn intake(&mut self, text: String) {
        if let Some(first) = self.steps.first_mut() {
            first.intake(text);
        }
    }

    fn output(&self) -> Option<String> {
        self.current().and_then(|s| s.output())
    }
}
