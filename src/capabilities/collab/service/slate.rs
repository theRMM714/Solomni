//! **代拟与确认、恢复与收尾**：提交需求（`set_task` 在 `collab.rs`）、代拟名单与确认（`draft_slate` / `confirm_slate`）、
//! 开工（`begin`）、断点续跑（`resume`）与终结判定（`is_done`）。

use super::collab::*;
use crate::capabilities::collab::service::discussion::Discussion;
use crate::capabilities::llm::api::{CompleteOpts, Llm};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Settings;
use crate::capabilities::session::api::SessionMeta;
use crate::capabilities::session::api::{LineView, Pending, SessionEvent};
use crate::capabilities::tools::api::ToolExec;
use crate::capabilities::workspace::api::Sandboxes;
use crate::capabilities::workspace::api::Workspace;
use std::sync::Arc;

use super::pump::*;
impl CollabSession {
    /// 委托代拟：核心拟发言名单（**拟名单业务**的用例，与「推荐模型」同一条 `slate` 协议），交用户确认。
    pub(crate) fn draft_slate(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        let roster = self.workspace.roster();
        let mut verify = self.core_verify_tools("planner");
        sink(crate::capabilities::session::api::working("核心"));
        let proposal = crate::capabilities::slate::api::propose(
            &crate::capabilities::slate::api::Parties {
                prompt: &*self.prompts,
                tools: &*self.systools,
                roster: &roster,
                agents: &self.settings.agents,
                models: &self.settings.models,
            },
            &mut crate::capabilities::slate::api::Request {
                task: &self.task,
                mode: crate::capabilities::slate::api::Mode::Collab,
                chat: self.core_chat.as_mut(),
                tool_mode: self.core_mode,
                opts: CompleteOpts::plain(false),
                cancel: None,
                verify: verify.as_mut(),
                sink,
            },
        );
        // 核心这一次调用结束了：交回"谁在干活"——下一棒（泵的下一步 / 等用户）会再推。
        sink(crate::capabilities::session::api::idle());
        let Ok(proposal) = proposal else {
            sink(SessionEvent::Notice(
                "[错误] 代拟失败（模型无响应格式）。请直接点名 agent。".into(),
            ));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        };
        // 拒收项如实告知（合法性由拟名单业务判，这里只转述）。
        for r in proposal.rejected {
            sink(SessionEvent::Notice(format!("[代拟] {}，拒收", r)));
        }
        if proposal.picks.is_empty() {
            sink(SessionEvent::Notice("[错误] 代拟名单无可用 agent".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        let line = self.view(LineView::system(
            "代拟",
            proposal
                .picks
                .iter()
                .map(|p| slate_item(&p.agent, &p.why))
                .collect::<Vec<_>>()
                .join("；"),
        ));
        sink(SessionEvent::Transcript(vec![line]));
        self.slate_picks = proposal.picks.into_iter().map(|p| p.agent).collect();
        self.ask_user(Pending::ConfirmSlate, sink);
    }

    /// 回应代拟名单确认（仅 ConfirmSlate 挂起时有效）。
    pub fn confirm_slate(&mut self, ok: bool, sink: &mut dyn FnMut(SessionEvent)) {
        let line = self.view(LineView::user(
            "名单",
            if ok { "确认" } else { "取消" }.to_string(),
        ));
        sink(SessionEvent::Transcript(vec![line]));
        if !ok {
            sink(SessionEvent::Notice("[取消] 已按用户意愿取消".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        if self.slate_picks.is_empty() {
            // 名单只活在内存里（落档发生在确认之后）；重启后回来会空手。
            // 与其拿着空名单开工，不如如实告知并重新拟一份（名单本来就是要用户过目的提案）。
            sink(SessionEvent::Notice(
                "[提示] 上次拟的名单未落档（重启会丢），重新拟一份，请再确认。".into(),
            ));
            self.draft_slate(sink);
            return;
        }
        self.roster = self.slate_picks.clone();
        sink(SessionEvent::Notice(format!(
            "[建组] {}",
            self.names().join(" + ")
        )));
        self.ask_user(Pending::ConfirmBegin, sink);
    }

    /// 确认开始讨论（allow = yes,allow 自裁授权）；开聊并一路泵到暂停或交付。
    pub fn begin(&mut self, allow: bool, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_some() {
            return;
        }
        self.allow = allow;
        let line = self.view(LineView::user(
            "开始",
            if allow { "yes,allow" } else { "yes" }.to_string(),
        ));
        sink(SessionEvent::Transcript(vec![line]));
        let prompts = self.prompts.clone();
        let (members, notes) = match self.assemble_members() {
            Ok(x) => x,
            Err(e) => {
                sink(SessionEvent::Notice(format!("[装配失败] {}", e)));
                return;
            }
        };
        for n in notes {
            sink(SessionEvent::Notice(n));
        }
        if self.core_is_demo {
            sink(SessionEvent::Notice(
                "[提示] 核心未配置供应商：整理/验收使用内置假模型（演示）".into(),
            ));
        }
        // 讨论也走**全局设置**（流式 + 预算），与单 agent 共用同一份。
        let llm = crate::capabilities::llm::api::LlmOpts {
            stream: self.settings.app.streaming,
            timeout_secs: self.settings.app.llm_timeout_secs,
        };
        // 讨论席的"协议"= **机制说明 + 讨论约定**：只说约定不说机制，AI 就不知道自己在什么流程里、
        // 该干什么（真机上就是空转）。
        // **能用哪些表态不在这里列**：核心按这一回合的身份注入工具块（engine::MemberTools::tools_block），
        // 清单与越权校验同源——同一份清单在提示词里再列一遍只会多一个会漂的地方。
        let protocol = format!(
            "{}\n{}",
            self.prompts.text(Segment::Mechanism),
            self.prompts.text(Segment::ChatProtocol)
        );
        let mut disc = Discussion::new(
            members,
            self.allow,
            prompts,
            Arc::clone(&self.systools),
            llm,
            std::sync::Arc::clone(&self.cancel),
            protocol,
        );
        // 开场**不在这里跑**：核心驱动（见 session-model.md 二之二）——这里只渲染提示词、置游标，
        // 下一步由核心取该 agent 的会话跑第一个回合（逐成员外送在 feed_with 里）。
        disc.start(&self.task);
        self.disc = Some(disc);
        self.pump_with(sink);
    }
}

impl CollabSession {
    /// 继续：从断点推进（协作不需要用户发言）。未开始时由用户经裁决门确认，不由继续代劳。
    pub fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done {
            return;
        }
        // 用户点「继续」= **重派核心指名没过的那几个节点**（只退这些；同阶段已通过的保持已通过，
        // 不整阶段重来）。放在这里而不是泵里：唤醒（子会话完成）不能替用户做这个决定。
        if let Some(Pending::NodeBlocked { nodes }) = self.pending.clone() {
            self.pending = None;
            self.gate_advice.clear();
            for n in &nodes {
                self.reset_node(n);
            }
        }
        if self.disc.is_some() {
            self.pump_with(sink);
            return;
        }
        if self.delegated && self.slate_picks.is_empty() && self.roster.is_empty() {
            self.draft_slate(sink);
        } else {
            sink(SessionEvent::Notice(
                "[提示] 等待你在裁决门确认名单 / 开始讨论。".into(),
            ));
        }
    }

    /// 撤回某 agent 的同意：转录追加一条撤回行（用户可见、也进上下文），并就地复位本轮表态。
    pub fn withdraw_agree(&mut self, agent: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let line = self.view(LineView::user("撤回", agent.to_string()));
        sink(SessionEvent::Transcript(vec![line]));
        if let Some(disc) = self.disc.as_mut() {
            for m in disc.members.iter_mut() {
                if m.id == agent {
                    m.agreed = false;
                }
            }
            disc.closed = false;
        }
    }

    /// 从落盘事件重建协作会话：名单取 meta.agents（权威），讨论进度由转录派生。
    /// 通道是可重建的机制，不是状态：按会话来时记住的 agent 名单重新装配。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        llm: Arc<dyn Llm + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        settings: Settings,
        prompts: Arc<dyn Prompt>,
        systools: Arc<dyn crate::capabilities::tools::api::Tools>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        meta: &SessionMeta,
        events: &[serde_json::Value],
        sandboxes: Sandboxes,
    ) -> Result<CollabSession, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let st = crate::capabilities::collab::domain::collab_state::derive(events, &names);
        // 全部已发出的转录行（按 id 顺序），连降级标记一起读回（样式靠它，不靠文案）。
        let mut all_lines: Vec<LineView> = Vec::new();
        for ev in events {
            if ev.get("type").and_then(|t| t.as_str()) == Some("transcript") {
                if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
                    for l in lines {
                        // **按线格式直接读回**：字段与落盘同源（说话人/动词/种类/思维链/工具视图都在）。
                        if let Ok(v) = serde_json::from_value::<LineView>(l.clone()) {
                            all_lines.push(v);
                        }
                    }
                }
            }
        }
        let total = all_lines.len() as u64;
        let core_channel = settings.core_channel();
        let (core_chat, core_is_demo) = llm.core_channel(core_channel.as_ref());
        // 形态要在 settings 被移进结构体之前算出来。
        let core_mode = settings.tool_mode_for(None);
        let mut s = CollabSession {
            delegated: meta.delegate,
            roster: meta.agents.clone(),
            task: st.task.clone().unwrap_or_default(),
            slate_picks: Vec::new(),
            settings,
            pending: None,
            allow: st.allow,
            plan: st.plan.clone(),
            // 链随 plan_review 事件落档：重建后按它还原，不重新整理（省一次模型调用）。
            chain: if st.chain.nodes.is_empty() {
                None
            } else {
                Some(st.chain.clone())
            },
            disc: None,
            turns: 0,
            turn_error: None,
            pending_ask: None,
            emitted: 0,
            next_line: total,
            reply_seq: crate::capabilities::session::api::max_reply(events),
            core_chat,
            core_is_demo,
            core_mode,
            prompts: prompts.clone(),
            systools: systools.clone(),
            llm,
            workspace,
            tools,
            log,
            spec: meta.exec.clone(),
            sandboxes,
            done: st.ended,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            plan_approved: st.plan_approved,
            gate_advice: String::new(),
        };
        if st.begun {
            // 讨论转录 = 最后一条 [用户:开始] 之后的行。
            let start = all_lines
                .iter()
                // 讨论转录 = 最后一条 [用户:开始] 之后的行。
                .rposition(|l| l.kind == "user" && l.verb == "开始")
                .map(|i| i + 1)
                .unwrap_or(all_lines.len());
            let disc_lines = all_lines[start..].to_vec();
            let (members, _) = s.assemble_members()?;
            let llm = s.llm_opts();
            // 恢复时与实时同一句：只有机制与约定，工具面随回合注入（见 tools_block）。
            let protocol = format!(
                "{}\n{}",
                s.prompts.text(Segment::Mechanism),
                s.prompts.text(Segment::ChatProtocol)
            );

            let mut disc = Discussion::new(
                members,
                st.allow,
                prompts,
                systools.clone(),
                llm,
                std::sync::Arc::clone(&s.cancel),
                protocol,
            );
            disc.round = st.round.max(1);
            disc.closed = st.closed;
            for m in disc.members.iter_mut() {
                m.present = st.present.get(&m.id).copied().unwrap_or(true);
                m.agreed = st.agreed.get(&m.id).copied().unwrap_or(false);
            }
            s.emitted = disc_lines.len();
            disc.transcript = disc_lines;
            s.disc = Some(disc);
        }
        s.pending = derive_pending(&st);
        Ok(s)
    }

    /// 会话是否已终结。
    pub fn is_done(&self) -> bool {
        self.done
    }
}
