//! 目的：协作会话作为**一个机制**：把泵的「推一步 / 让出一回合 / 交回」翻译成机制面的动作。
//! 管：`Mechanism for CollabSession` 的逐项实现（成员抽问、交回处置、失败记下）。
//! 不管：泵与状态机本身（在 pump / discussion / turn_io）；驱动循环（在 turn）。
//! 联动：`turn` 的驱动循环与 `turn::generation` 的工作线程只认 `Mechanism`，认不出这是协作。

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::capabilities::conversation::collab::service::collab::CollabSession;
use crate::capabilities::conversation::collab::service::discussion::AfterTurn;
use crate::capabilities::conversation::session::api::{
    Handback, Mechanism, MemberAsk, MemberTurn, SessionEvent,
};

impl Mechanism for CollabSession {
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
            identity,
            turn,
            turn_id: self.next_turn_id(),
            round: self.round(),
            systools: self.systools(),
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn next_line(&self) -> u64 {
        self.next_line
    }

    fn set_next_line(&mut self, next: u64) {
        self.next_line = next;
    }

    fn is_done(&self) -> bool {
        CollabSession::is_done(self)
    }

    fn notice(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        CollabSession::notice(self, text, sink);
    }
}
