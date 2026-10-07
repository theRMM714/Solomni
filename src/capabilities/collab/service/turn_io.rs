//! **回合收发**：把成员的回复喂回状态机（`feed_with`）、取出待问的一步（`take_ask`）、
//! 回答与裁决（`answer` / `decide`）以及核心核实工具面与通道参数。

use super::collab::*;
use crate::capabilities::collab::service::discussion::TurnOut;
use crate::capabilities::session::api::{LineView, Pending, SessionEvent};
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
            fence: crate::capabilities::tools::api::FenceSpec::from_sandbox(&sb, false),
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

    /// 回答 ask（仅 Ask 挂起时有效）；回答转达后继续泵。用户回答同样先改写 @ 引用。
    pub fn answer(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        if matches!(self.pending, Some(Pending::Ask { .. })) {
            self.pending = None;
            self.gate_advice.clear();
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
    }

    /// 用户对当前裁决的**自由文本回应**（与"二选一确认"分开）：
    /// - 请教：他的话进**主会话**（所有成员下一回合都看得到），继续泵；**不单独转给那个成员**。
    /// - 方案待审：先记下他的话（进主会话），再过审开工。
    /// - 节点没过：先记下他的话，再重派没过的节点。
    /// - 名单 / 开始是二选一（前端给的是确认按钮），不走这条路——如实说明，不假装收下。
    ///
    /// 没有挂起的事同样如实说。
    pub fn decide(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        match self.pending.clone() {
            // 请教：他的话进主会话（所有成员下一回合都看得到），继续泵。
            // 这不是"放行工作"，所以**不判明确性**——他说什么就是什么。
            Some(Pending::Ask { .. }) => self.answer(text, sink),
            // 放行类（方案待审 / 节点没过）：**由核心 AI 判定他的意图是否明确**，明确才开工/放行。
            // 不明确就不开工（他的话仍进主会话当反馈，关卡留着等他补一句）。
            Some(p @ Pending::PlanReview) | Some(p @ Pending::NodeBlocked { .. }) => {
                let kind = p.decision_parts().0;
                let brief = self.decision_brief(&p);
                let text_owned = text.to_string();
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
                    &text_owned,
                    sink,
                );
                sink(crate::capabilities::session::api::idle());
                match judged {
                    Ok((true, why)) => {
                        self.note_user(text, sink);
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
                            }
                            _ => self.resume(sink),
                        }
                    }
                    // 不明确 = **不开工**：不自动重试、不自己往下推，等他补一句。
                    Ok((false, why)) => {
                        self.note_user(text, sink);
                        sink(SessionEvent::Notice(if why.trim().is_empty() {
                            "[裁决] 我还没听出明确的意思，先不开工；请再说一句（要做 / 不要做 / 照哪个走）。"
                                .to_string()
                        } else {
                            format!("[裁决] 先不开工——{}；请再说一句。", why)
                        }));
                    }
                    Err(err) => {
                        self.note_user(text, sink);
                        sink(SessionEvent::Notice(
                            crate::capabilities::session::api::interrupted_note(&format!(
                                "判定你的意思时没能问模型（{}）；为稳妥先不开工，请再说一句。",
                                err
                            )),
                        ));
                    }
                }
            }
            Some(Pending::ConfirmSlate) | Some(Pending::ConfirmBegin) => {
                sink(SessionEvent::Notice(
                    "[裁决] 这一步是二选一（确认 / 取消），请用卡片上的按钮。".to_string(),
                ));
            }
            None => sink(SessionEvent::Notice(
                "[裁决] 现在没有等你定的事。".to_string(),
            )),
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
