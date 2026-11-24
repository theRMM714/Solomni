//! 目的：协作会话作为**一个流程**：把泵的「推一步 / 让出一回合 / 交回」翻译成流程面的动作。
//! 管：`Flow for CollabSession` 的逐项实现（成员抽问、交回处置、失败记下）。
//! 不管：泵与状态机本身（在 pump / discussion / turn_io）；驱动循环（在 turn）。
//! 联动：`turn` 的驱动循环与 `turn::generation` 的工作线程只认 `Flow`，认不出这是协作。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::conversation::collab::domain::events::CollabEvent;
use crate::capabilities::conversation::collab::domain::gates::{OPT_BEGIN_ALLOW, Pending};
use crate::capabilities::conversation::collab::service::collab::{CollabSession, Phase};
use crate::capabilities::conversation::collab::service::discussion::AfterTurn;
use crate::capabilities::conversation::session::api::{
    Flow, Gate, GateTicket, Handback, MemberAsk, MemberTurn, SessionEvent, Settle, WorkPlan,
    mechanism_of, seat,
};
use crate::capabilities::taskchain::api::NodeStatus;
use crate::capabilities::workspace::api::Sandboxes;

impl Flow for CollabSession {
    fn set_cancel(&mut self, cancel: Arc<AtomicBool>) {
        CollabSession::set_cancel(self, cancel);
    }

    fn start(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.start_if_needed();
        self.pump_with(sink);
    }

    fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        CollabSession::resume(self, sink);
    }

    fn step(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        self.start_if_needed();
        self.pump_with(sink);
    }

    fn take_ask(&mut self) -> Option<MemberAsk> {
        let (index, identity, turn) = self.take_pending_ask()?;
        let member = self.member_id(index)?;
        Some(MemberAsk {
            index,
            member,
            seat: self.member_seat_at(index)?,
            identity,
            turn,
            turn_id: self.next_turn_id(),
            round: self.round(),
            opts: self.disc_opts(),
        })
    }

    fn handback(&mut self, ask: &MemberAsk, turn: &MemberTurn) -> Handback {
        let cap = self.settings.app.discuss_remind_cap;
        match self.after_member_turn(ask.index, turn.verb.is_some(), self.cancelled(), cap) {
            AfterTurn::Done => Handback::Fed,
            AfterTurn::Remind => Handback::Remind(self.reminder_text()),
            AfterTurn::Unanswered => Handback::PassOver,
        }
    }

    fn feed(&mut self, ask: &MemberAsk, turn: MemberTurn, sink: &mut dyn FnMut(SessionEvent)) {
        self.feed_with(ask.index, turn, ask.turn_id, sink);
    }

    fn pass_over(&mut self, ask: &MemberAsk, sink: &mut dyn FnMut(SessionEvent)) {
        CollabSession::pass_over(self, ask.index, sink);
    }

    fn failed(&mut self, err: &str) {
        self.note_turn_failure(err.to_string());
    }

    fn next_line(&self) -> u64 {
        self.next_line
    }

    fn set_next_line(&mut self, next: u64) {
        self.next_line = next;
    }

    fn adopt_roster(&mut self, roster: &[AgentMeta]) {
        // 名单是会话级事实：代拟的名单刚被确认时，后面那一段也要认得出谁能出演。
        self.roster = roster.to_vec();
    }

    fn intake(&mut self, text: String) {
        // 上一段交来的产物就是这一段的需求（讨论段的总结 → 执行段要做的需求）。
        self.task = text;
    }

    fn begin_with(&mut self, task: &str, sink: &mut dyn FnMut(SessionEvent)) {
        // 第一段收用户需求：需求入转录并按分段规则起步（讨论段开会 / 执行段直接拟计划）。
        CollabSession::set_task(self, task, sink);
    }

    fn output(&self) -> Option<String> {
        self.summary.clone()
    }

    fn is_done(&self) -> bool {
        CollabSession::is_done(self)
    }

    fn notice(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        CollabSession::notice(self, text, sink);
    }

    fn settle(
        &mut self,
        ticket: &GateTicket,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<Settle, String> {
        let (resume, roster) = CollabSession::dispose(self, ticket, sink)?;
        Ok(Settle { resume, roster })
    }

    fn settles_inline(&self, gate: &Gate) -> bool {
        matches!(Pending::from_gate(gate), Some((Pending::ConfirmSlate, _)))
    }

    fn plan(&self) -> Option<&dyn WorkPlan> {
        Some(self)
    }

    fn plan_mut(&mut self) -> Option<&mut dyn WorkPlan> {
        Some(self)
    }

    fn withdraw_agree(&mut self, agent: &str, sink: &mut dyn FnMut(SessionEvent)) -> bool {
        CollabSession::withdraw_agree(self, agent, sink);
        true
    }

    fn adapt_text(&self, text: &str) -> String {
        self.retask_text(text)
    }

    fn set_sandboxes(&mut self, sandboxes: Sandboxes) {
        CollabSession::set_sandboxes(self, sandboxes);
    }

    fn turn_cursor(&mut self) -> u64 {
        self.next_turn_id()
    }

    fn default_answer(&self) -> Option<String> {
        // 核心自己建的子会话停在「允许自裁」那张卡上时默认按它答（选项 id 是协作的词汇，驱动不认）。
        Some(OPT_BEGIN_ALLOW.to_string())
    }

    fn member_seat(&self) -> Option<&'static str> {
        let id = match self.phase {
            Phase::Discuss => "discuss",
            Phase::Execute => "execute",
        };
        mechanism_of(id)?
            .seats
            .iter()
            .filter_map(|id| seat(id))
            .find(|s| !s.core)
            .map(|s| s.id)
    }
}

impl WorkPlan for CollabSession {
    fn approved(&self) -> bool {
        self.plan_approved()
    }

    fn ready(&self) -> Vec<(String, String)> {
        let Some(chain) = self.chain() else {
            return Vec::new();
        };
        // **只派当前阶段**：同一阶段的节点并发跑，下一阶段要等本阶段整体验收通过。
        let stage = chain.current_stage().unwrap_or(1);
        chain
            .stage_ready(stage)
            .into_iter()
            .filter(|n| n.sub_session.is_none())
            .map(|n| (n.id.clone(), n.assignee.clone()))
            .collect()
    }

    fn busy(&self) -> Vec<String> {
        let Some(chain) = self.chain() else {
            return Vec::new();
        };
        chain
            .nodes
            .iter()
            .filter(|n| matches!(n.status, NodeStatus::Running))
            .map(|n| n.assignee.clone())
            .collect()
    }

    fn objective(&self, node: &str) -> Option<(String, String)> {
        let chain = self.chain()?;
        let n = chain.nodes.iter().find(|n| n.id == node)?;
        let note = n
            .acceptance
            .as_ref()
            .map(|a| a.note.clone())
            .unwrap_or_default();
        // **返工必须知道上次错在哪**，否则只会原样重做。
        let rework = if note.trim().is_empty() {
            String::new()
        } else {
            format!(
                "\n== 上次没通过的原因 ==\n{}\n这次请针对上面的原因返工。\n",
                note
            )
        };
        Some((n.objective.clone(), rework))
    }

    fn assignee(&self, node: &str) -> Option<String> {
        self.chain()?
            .nodes
            .iter()
            .find(|n| n.id == node)
            .map(|n| n.assignee.clone())
    }

    fn running(&self, sid: &str) -> Option<String> {
        self.running_node_of(sid)
    }

    fn start(&mut self, node: &str, sid: &str) {
        self.mark_node_started(node, sid);
    }

    fn finish(&mut self, node: &str, note: &str) {
        self.mark_node_done(node, note);
    }

    fn started_event(&self, node: &str, sid: &str, assignee: &str) -> SessionEvent {
        CollabEvent::NodeStarted {
            node: node.to_string(),
            sid: sid.to_string(),
            assignee: assignee.to_string(),
        }
        .push()
    }
}
