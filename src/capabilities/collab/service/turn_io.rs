//! **回合收发**：把成员的回复喂回状态机（`feed_with`）、取出待问的一步（`take_ask`）、
//! 把一张已出队的回答落到业务上（`dispose`）以及核心核实工具面与通道参数。

use super::collab::*;
use crate::capabilities::collab::service::discussion::TurnOut;
use crate::capabilities::session::api::{
    AgentMeta, LineView, Pending, SessionEvent, OPT_ASK_REPLY, OPT_BEGIN, OPT_BEGIN_ALLOW,
    OPT_NODE_REWORK, OPT_NODE_SAY, OPT_PLAN_SAY, OPT_PLAN_START, OPT_SLATE_CANCEL,
    OPT_SLATE_CONFIRM,
};
use std::sync::Arc;

use super::pump::*;
impl CollabSession {
    /// 核心把某个成员回合的结果**交回来**：吸收、外送、继续泵（驱动权在核心，见 session-model.md 二之二）。
    /// 调用前核心应把该回合的核实行落进**该 agent 自己的会话**（它们不属于主会话）。
    pub fn feed_with(
        &mut self,
        i: usize,
        turn: crate::capabilities::collab::service::discussion::MemberTurn,
        turn_id: u64,
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        let next_line = std::cell::Cell::new(self.next_line);
        let handed = std::cell::Cell::new(0usize);
        let mut on_lines = |lines: &[LineView], s: &mut dyn FnMut(SessionEvent)| {
            emit_new_lines(lines, &next_line, &handed, s);
        };
        let out = self.disc.as_mut().expect("disc 已确认存在").feed(
            i,
            turn,
            turn_id,
            &mut on_lines,
            sink,
        );
        self.next_line = next_line.get();
        self.emitted += handed.get();
        // 兜底：feed 提前返回时把剩下的行补齐；已交出去过的不会再出。
        if let Some(d) = self.disc.as_ref() {
            push_delta(d, &mut self.emitted, &mut self.next_line, sink);
        }
        if let Some(TurnOut::AskUser { member, question }) = out {
            self.ask_user(Pending::Ask { member, question }, sink);
            return;
        }
        self.pump_with(sink);
    }

    /// 泵让出的那一步（该问谁、给它什么上下文）——由核心取走并驱动。
    pub fn take_ask(&mut self) -> Option<(usize, String, Vec<crate::capabilities::llm::api::Msg>)> {
        self.pending_ask.take()
    }

    /// 第 i 个成员的 agent 名（核心据此拼出它的会话名 <工作>--<agent>）。
    pub fn member_id(&self, i: usize) -> Option<String> {
        self.disc.as_ref()?.member_id(i).map(|s| s.to_string())
    }

    /// 核心核实用的小工具环境：**只读**、根是本次工作的共享区。
    /// 为什么要它：核心操作（出方案 / 节点验收…）也常需要"先看看现场再下结论"，
    /// 而核心不是 member、手里没有工具环境——没有它，模型一想核实就被判"没调用 X"而整步中断。
    /// 工具面只发**该角色的只读核实工具**（按声明里的 capability = fs-read 判定），写类一律不发。
    pub(crate) fn core_verify_tools(
        &self,
        role: &str,
    ) -> Option<crate::capabilities::session::api::MemberTools> {
        let mut sb = self.sandboxes.list.first()?.clone();
        sb.agent = "核心".to_string();
        sb.private = sb.shared.clone();
        sb.modules.clear();
        sb.modules_with_userdata.clear();
        let allowed: Vec<String> = self
            .systools
            .tool_face(role)
            .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
            .unwrap_or_default();
        Some(crate::capabilities::session::api::MemberTools {
            mode: self.core_mode,
            role: role.to_string(),
            modules: std::collections::BTreeMap::new(),
            observations: crate::capabilities::tools::api::Observations::default(),
            llm: Arc::clone(&self.llm),
            log: Arc::clone(&self.log),
            tools: Arc::clone(&self.tools),
            sandbox: sb.clone(),
            builtin_tools: self.systools.book(),
            unavailable: std::collections::BTreeMap::new(),
            fence: crate::kernel::api::FenceSpec::from_sandbox(&sb, false),
            reply_seq: 0,
            line: Default::default(),
            allowed,
            with_modules: false,
            notes: crate::capabilities::tools::api::ToolNotes::default(),
            handlers: Vec::new(),
        })
    }

    /// 讨论回合的**工具面**（动词 + 只读核实）：核心驱动时交给 turn_with。
    pub fn systools(&self) -> Arc<dyn crate::capabilities::tools::api::Tools> {
        Arc::clone(&self.systools)
    }

    /// 「停止」标志：与核心共享同一个（停止能在一个模型调用内收尾）。
    pub fn disc_cancel(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.disc
            .as_ref()
            .map(|d| d.cancel_flag())
            .unwrap_or_else(|| std::sync::Arc::clone(&self.cancel))
    }

    /// 本回合的调用选项（流式 + 预算，取全局设置）。
    pub fn disc_opts(&self) -> crate::capabilities::llm::api::CompleteOpts<'static> {
        crate::capabilities::llm::api::CompleteOpts::plain(self.settings.app.streaming)
            .with_timeout(self.settings.app.llm_timeout_secs)
    }

    /// 开场还没开始过就先开始（核心驱动的第一步）。
    pub fn start_if_needed(&mut self) {
        if let Some(d) = self.disc.as_mut() {
            if d.not_started() {
                d.start(&self.task);
            }
        }
    }

    /// 回答请教那一关：用户的话进主会话（所有成员下一回合都看得到），接着往下推。
    /// 约束：那一关已由队列出队——这里只管"他的话怎么进业务"，不碰队列。
    pub(crate) fn answer(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let roots = crate::capabilities::prompt::api::RefRoots {
            work: self.sandboxes.shared.clone(),
            private: None,
        };
        let text =
            crate::capabilities::prompt::api::rewrite(text, None, &roots, &self.prompts.refs());
        if let Some(disc) = self.disc.as_mut() {
            disc.pending_user_answers.push(text);
        }
        self.pump_with(sink);
    }

    /// 目的：把一张**已经出队**的回答落到业务上（处置归发起方）：按选项 id 分派这一关该做什么。
    /// 参数：`ticket` = 队列给的处置票（卡号 / 选项 / 附言 / 这一关的材料与建议）。
    /// 返回：这一关**放行了没有**（true = 开工 / 重派，调用方要接着跑整条流水线）+ 刚定下的名单。
    /// 错误：附言必填的那几关没给附言、选项不在这一关能分派的范围里，都如实拒绝。
    pub fn dispose(
        &mut self,
        ticket: &crate::capabilities::session::api::GateTicket,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<(bool, Option<Vec<AgentMeta>>), String> {
        let option = ticket.option.as_str();
        let note = ticket.note.trim().to_string();
        let empty_before = self.roster.is_empty();
        let p = ticket.pending.clone();
        let nodes = match &p {
            Pending::NodeBlocked { nodes } => nodes.clone(),
            _ => Vec::new(),
        };
        let go = match option {
            // 请教：他的话进主会话（所有成员下一回合都看得到），继续泵；不单独转给那个成员。
            OPT_ASK_REPLY => {
                self.answer(&note, sink);
                false
            }
            OPT_SLATE_CONFIRM | OPT_SLATE_CANCEL => {
                self.confirm_slate(option == OPT_SLATE_CONFIRM, sink);
                false
            }
            OPT_BEGIN | OPT_BEGIN_ALLOW => {
                self.begin(option == OPT_BEGIN_ALLOW, sink);
                false
            }
            // 放行类：他的附言进主会话当反馈，然后才开工 / 重派。
            OPT_PLAN_START => {
                self.note_user(&note, sink);
                self.approve_plan(sink);
                self.resume(sink);
                true
            }
            OPT_NODE_REWORK => {
                self.note_user(&note, sink);
                for n in &nodes {
                    self.reset_node(n);
                }
                self.resume(sink);
                true
            }
            // "先说一句"：他的话进主会话当反馈，**由核心 AI 判这句话是否明确**——
            // 明确才开工 / 重派，模糊就不动、关卡继续挂着（见 session-model.md「请用户裁决」）。
            OPT_PLAN_SAY | OPT_NODE_SAY => self.judge_note(&p, &ticket.advice, &note, sink),
            other => return Err(format!("这张卡上没有这个选项：{}", other)),
        };
        // 名单刚由这一答定下来：调用方要把 roster 写回 meta（重建与沙箱归属都读它）。
        let confirmed = (empty_before && !self.roster.is_empty()).then(|| self.roster.to_vec());
        Ok((go, confirmed))
    }

    /// "先说一句"这一条的处理：他的话进主会话当反馈，再由核心 AI 判**是否明确**。
    /// 明确才开工 / 重派；模糊就不动、关卡继续挂着（这一条已经答过，续一张新的接着问）。
    /// 返回：明确到可以放行 = true。
    /// 参数：advice = 原来那一关带着的建议（续的新卡要带上它，别让建议随出队丢掉）。
    fn judge_note(
        &mut self,
        p: &Pending,
        advice: &str,
        note: &str,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> bool {
        let kind = p.kind();
        let brief = self.decision_brief(p);
        let text = note.to_string();
        let mut verify = self.core_verify_tools("planner");
        sink(crate::capabilities::session::api::working("核心"));
        let judged = Self::judge_clear(
            &*self.prompts,
            &*self.systools,
            &self.cancel,
            crate::capabilities::llm::api::CompleteOpts::plain(self.settings.app.streaming)
                .with_timeout(self.settings.app.llm_timeout_secs),
            self.core_mode,
            self.core_chat.as_mut(),
            verify.as_mut(),
            kind,
            &brief,
            &text,
            sink,
        );
        sink(crate::capabilities::session::api::idle());
        self.note_user(note, sink);
        match judged {
            Ok((true, why)) => {
                if !why.trim().is_empty() {
                    sink(SessionEvent::Notice(format!(
                        "[裁决] 照你说的开工：{}",
                        why
                    )));
                }
                match p {
                    Pending::PlanReview => {
                        self.approve_plan(sink);
                        self.resume(sink);
                        true
                    }
                    Pending::NodeBlocked { nodes } => {
                        for n in nodes {
                            self.reset_node(n);
                        }
                        self.resume(sink);
                        true
                    }
                    _ => false,
                }
            }
            // 不明确 = **不动**：不自动往下推，这一关续一张新卡接着等他补一句。
            Ok((false, why)) => {
                sink(SessionEvent::Notice(if why.trim().is_empty() {
                    "[裁决] 我还没听出明确的意思，先不动；请再说一句（要做 / 不要做 / 照哪个走）。"
                        .to_string()
                } else {
                    format!("[裁决] 先不动——{}；请再说一句。", why)
                }));
                self.gate_advice = advice.to_string();
                self.ask_user(p.clone(), sink);
                false
            }
            Err(err) => {
                sink(SessionEvent::Notice(
                    crate::capabilities::session::api::interrupted_note(&format!(
                        "判定你的意思时没能问模型（{}）；为稳妥先不动，请再说一句。",
                        err
                    )),
                ));
                self.gate_advice = advice.to_string();
                self.ask_user(p.clone(), sink);
                false
            }
        }
    }

    /// 用户的话进**主会话转录**（所有成员的下一回合都看得到）。空话不记。
    pub(crate) fn note_user(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let roots = crate::capabilities::prompt::api::RefRoots {
            work: self.sandboxes.shared.clone(),
            private: None,
        };
        let text =
            crate::capabilities::prompt::api::rewrite(text, None, &roots, &self.prompts.refs());
        let line = self.view(LineView::user("", text));
        sink(SessionEvent::Transcript(vec![line]));
    }
}
