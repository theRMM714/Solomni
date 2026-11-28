//! 目的：复合流程——装配期把若干段流程串成一条：前一段收尾就把产物交给下一段（intake），转录行号接上。
//! 管：按序推进、交接产物与行号、把每段的进度写成落盘事实；把流程面整套转给**当前这一段**。
//! 不管：任何一段自己的状态机（归各流程）；会话类型表与装配（归会话单元的装配与组合根）。
//! 联动：`systools/profiles.yaml` 里一条会话类型的 flows 序列由它承载；驱动只认 `Flow`。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::kernel::api::types::Ask;
use crate::units::business::agentloop::api::MemberTurn;
use crate::units::business::session_model::api::{Artifact, Kind};
use crate::units::business::transcript::api::SessionEvent;
use crate::units::business::turn_state::domain::decisions::GateTicket;
use crate::units::business::turn_state::domain::flow::{
    Flow, Handback, MemberAsk, Settle, WorkPlan,
};
use crate::units::business::turn_state::domain::stage;
use crate::units::business::workbench::api::Benches;
use crate::units::pure::registry_model::api::AgentMeta;

/// 目的：一段在声明里的产物契约（推进时对照：这一段交什么、下一段收什么）。
#[derive(Debug, Clone)]
pub struct StepDecl {
    /// 目的：段 id（文案点名用）。
    pub id: String,
    /// 目的：这一段收得下的产物种类。
    pub accepts: Vec<Kind>,
    /// 目的：这一段声明交出的产物种类（`None` = 不交）。
    pub produces: Option<Kind>,
}

/// 目的：一串按顺序跑的流程段：前一段收尾后产物交给下一段，行号接着上一段。
/// 约束：恢复时只造起跑点之后的段（`origin` 是它们在整条流水线里的绝对位置）。
pub struct Sequence {
    steps: Vec<Box<dyn Flow>>,
    decls: Vec<StepDecl>,
    origin: usize,
    at: usize,
    pending: Vec<SessionEvent>,
}

impl Sequence {
    /// 目的：恢复出来的流水线：`steps` 从绝对第 `origin` 段开始，`line` 是这一段的行号基线，
    ///   `handoff` 是切点交给它的产物（老段已收尾，不再造也不再走一遍）。
    pub fn resumed(
        steps: Vec<Box<dyn Flow>>,
        decls: Vec<StepDecl>,
        origin: usize,
        line: u64,
        handoff: Option<Artifact>,
    ) -> Sequence {
        let mut seq = Sequence {
            steps,
            decls,
            origin,
            at: 0,
            pending: Vec::new(),
        };
        if let Some(first) = seq.steps.first_mut() {
            first.set_next_line(line);
            if let Some(artifact) = handoff {
                first.intake(artifact);
            }
        }
        seq
    }

    fn current(&self) -> Option<&dyn Flow> {
        self.steps.get(self.at).map(|s| &**s as &dyn Flow)
    }

    fn current_mut(&mut self) -> Option<&mut (dyn Flow + 'static)> {
        self.steps
            .get_mut(self.at)
            .map(|s| &mut **s as &mut dyn Flow)
    }

    /// 目的：交出产物前先对照声明：这一段交的种类要在自己的 `produces` 里，下一段要收得下。
    /// 返回：`Some(原因)` = 不许推进（原因会记成提示，停在原地）；`None` = 可以交。
    fn handoff_problem(&self, made: Option<&Artifact>) -> Option<String> {
        let artifact = made?;
        let kind = artifact.kind_of();
        let cur = self.decls.get(self.at)?;
        match cur.produces {
            Some(declared) if declared == kind => {}
            Some(declared) => {
                return Some(format!(
                    "流程段 {} 声明交出{}，实际交出的是{}：停在原地，不往下走（systools/loops.yaml）",
                    cur.id,
                    declared.name(),
                    kind.name()
                ));
            }
            None => {
                return Some(format!(
                    "流程段 {} 没声明产物，却交出了{}：停在原地，不往下走（systools/loops.yaml）",
                    cur.id,
                    kind.name()
                ));
            }
        }
        let next = self.decls.get(self.at + 1)?;
        if !next.accepts.contains(&kind) {
            return Some(format!(
                "下一段 {} 不收{}（accepts 里没有它）：停在原地，不往下走（systools/loops.yaml）",
                next.id,
                kind.name()
            ));
        }
        None
    }

    /// 目的：当前这一段已收尾就往下一段交：产物给 intake，行号接着上一段，并写下进度事实。
    fn settle_forward(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        while self.at + 1 < self.steps.len() && self.steps[self.at].is_done() {
            let line = self.steps[self.at].next_line();
            let made = self.steps[self.at].output();
            if let Some(why) = self.handoff_problem(made.as_ref()) {
                sink(SessionEvent::Notice(why));
                return;
            }
            self.at += 1;
            sink(stage::mark(self.origin + self.at, line, made.as_ref()));
            let next = &mut self.steps[self.at];
            next.set_next_line(line);
            if let Some(artifact) = made {
                // 原样搬运：组合子不解释产物的内容，接不接得住由声明与装配期校验管。
                next.intake(artifact);
            }
        }
    }

    /// 目的：把还没有 sink 时攒下的进度事件交出去（`take_ask` 没有 sink，先存着）。
    fn flush(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        for ev in std::mem::take(&mut self.pending) {
            sink(ev);
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
        self.flush(sink);
        self.settle_forward(sink);
        if let Some(step) = self.current_mut() {
            step.start(sink);
        }
    }

    fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.flush(sink);
        self.settle_forward(sink);
        if let Some(step) = self.current_mut() {
            step.resume(sink);
        }
    }

    fn step(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.flush(sink);
        self.settle_forward(sink);
        if let Some(step) = self.current_mut() {
            step.step(sink);
        }
    }

    fn take_ask(&mut self) -> Option<MemberAsk> {
        // 这里没有 sink：推进产生的进度事件先攒着，下一次有人收事件时再交出去。
        let mut held = Vec::new();
        self.settle_forward(&mut |ev| held.push(ev));
        self.pending.extend(held);
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
        // 需求只交给当前这一段：恢复出来的流水线当前段就是起跑点那一段。
        self.flush(sink);
        if let Some(step) = self.current_mut() {
            step.begin_with(task, sink);
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

    fn set_sandboxes(&mut self, sandboxes: Benches) {
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

    fn member_identity(&self) -> Option<(String, String)> {
        self.current().and_then(|s| s.member_identity())
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
        self.flush(sink);
        self.settle_forward(sink);
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
        Err(format!("这张卡不是本会话的流程发的：{}", ticket.ask.kind))
    }

    fn settles_inline(&self, ask: &Ask) -> bool {
        self.steps.iter().any(|s| s.settles_inline(ask))
    }

    fn intake(&mut self, artifact: Artifact) {
        // 交给本段（复合流程嵌在别人里面时，收到的就是当前这一段的输入）。
        if let Some(step) = self.current_mut() {
            step.intake(artifact);
        }
    }

    fn output(&self) -> Option<Artifact> {
        self.current().and_then(|s| s.output())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：测试替身：一段只会收尾 / 交产物 / 记下收到了什么的流程。
    struct Fake {
        done: bool,
        out: Option<Artifact>,
        got: Arc<std::sync::Mutex<Vec<Artifact>>>,
        next: u64,
    }

    impl Fake {
        fn new(done: bool, out: Option<Artifact>) -> (Fake, Arc<std::sync::Mutex<Vec<Artifact>>>) {
            let got = Arc::new(std::sync::Mutex::new(Vec::new()));
            (
                Fake {
                    done,
                    out,
                    got: Arc::clone(&got),
                    next: 0,
                },
                got,
            )
        }
    }

    impl Flow for Fake {
        fn set_cancel(&mut self, _cancel: Arc<AtomicBool>) {}
        fn start(&mut self, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn resume(&mut self, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn step(&mut self, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn take_ask(&mut self) -> Option<MemberAsk> {
            None
        }
        fn handback(&mut self, _ask: &MemberAsk, _turn: &MemberTurn) -> Handback {
            Handback::PassOver
        }
        fn feed(
            &mut self,
            _ask: &MemberAsk,
            _turn: MemberTurn,
            _sink: &mut dyn FnMut(SessionEvent),
        ) {
        }
        fn pass_over(&mut self, _ask: &MemberAsk, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn failed(&mut self, _err: &str) {}
        fn next_line(&self) -> u64 {
            self.next
        }
        fn set_next_line(&mut self, next: u64) {
            self.next = next;
        }
        fn intake(&mut self, artifact: Artifact) {
            self.got.lock().unwrap().push(artifact);
        }
        fn output(&self) -> Option<Artifact> {
            self.out.clone()
        }
        fn is_done(&self) -> bool {
            self.done
        }
    }

    fn decl(id: &str, accepts: &[Kind], produces: Option<Kind>) -> StepDecl {
        StepDecl {
            id: id.to_string(),
            accepts: accepts.to_vec(),
            produces,
        }
    }

    /// 目的：前一段收尾就把产物原样交给下一段。
    #[test]
    fn the_composite_hands_the_artifact_over_untouched() {
        let (first, _a) = Fake::new(true, Some(Artifact::Summary("总结".to_string())));
        let (second, b) = Fake::new(false, None);
        let mut seq = Sequence::resumed(
            vec![Box::new(first), Box::new(second)],
            Vec::new(),
            0,
            0,
            None,
        );
        seq.start(&mut |_| {});
        assert_eq!(
            b.lock().unwrap().as_slice(),
            &[Artifact::Summary("总结".to_string())]
        );
        assert_eq!(seq.next_line(), 0);
        assert!(!seq.is_done());
    }

    /// 目的：需求交给当前这一段收下（恢复出来的流水线也一样交给起跑点那段）。
    #[test]
    fn an_incoming_requirement_lands_on_the_first_step() {
        let (first, a) = Fake::new(false, None);
        let (second, _b) = Fake::new(false, None);
        let mut seq = Sequence::resumed(
            vec![Box::new(first), Box::new(second)],
            Vec::new(),
            0,
            0,
            None,
        );
        seq.begin_with("需求", &mut |_| {});
        assert_eq!(
            a.lock().unwrap().as_slice(),
            &[Artifact::Requirement("需求".to_string())]
        );
    }

    /// 目的：推进一段就在转录里落下一条进度事实（切点行号与产物都在里面）。
    #[test]
    fn advancing_leaves_a_stage_fact() {
        let (first, _a) = Fake::new(true, Some(Artifact::Summary("总结".to_string())));
        let (second, b) = Fake::new(false, None);
        let decls = vec![
            decl("discuss", &[Kind::Requirement], Some(Kind::Summary)),
            decl("execute", &[Kind::Summary], None),
        ];
        let mut seq =
            Sequence::resumed(vec![Box::new(first), Box::new(second)], decls, 0, 12, None);
        let mut seen = Vec::new();
        seq.start(&mut |ev| seen.push(ev.to_json()));
        let mark = seen
            .iter()
            .find(|ev| ev.get("type").and_then(|t| t.as_str()) == Some("stage"))
            .expect("推进一段要写一条进度事实");
        assert_eq!(mark.get("path").and_then(|p| p.as_str()), Some("seq[1]"));
        assert_eq!(mark.get("at").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(mark.get("line").and_then(|v| v.as_u64()), Some(12));
        assert_eq!(b.lock().unwrap().len(), 1, "产物要交给下一段");
    }

    /// 目的：下一段收不下这个种类就停在原地，不写进度事实、也不把产物塞进去。
    #[test]
    fn a_handoff_the_next_step_does_not_take_stops_right_here() {
        let (first, _a) = Fake::new(true, Some(Artifact::Summary("总结".to_string())));
        let (second, b) = Fake::new(false, None);
        let decls = vec![
            decl("discuss", &[Kind::Requirement], Some(Kind::Summary)),
            decl("execute", &[Kind::Requirement], None),
        ];
        let mut seq = Sequence::resumed(vec![Box::new(first), Box::new(second)], decls, 0, 0, None);
        let mut seen = Vec::new();
        seq.start(&mut |ev| seen.push(ev.to_json()));
        assert!(
            seen.iter()
                .all(|ev| ev.get("type").and_then(|t| t.as_str()) != Some("stage")),
            "不许推进"
        );
        let notice = seen
            .iter()
            .find(|ev| ev.get("type").and_then(|t| t.as_str()) == Some("notice"))
            .expect("要给出停在原地的原因");
        assert!(notice.to_string().contains("不收"), "{}", notice);
        assert!(b.lock().unwrap().is_empty());
    }
}
