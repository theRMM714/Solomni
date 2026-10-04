//! **讨论泵**：把讨论推进一步（`pump_with`）并装配成员（`assemble_members`）。
//!
//! 泵不自己调模型：它把"该问谁、带什么上下文"交回协调业务（见 session-model.md 二）。
//! 行外送与增量（`emit_new_lines` / `push_delta` / `review_event` / `derive_pending`）也在这里。

use super::collab::*;
use crate::capabilities::collab::service::discussion::{Discussion, Member, TurnOut, MAX_ROUNDS};
use crate::capabilities::collab::service::synthesis::Execution;
use crate::capabilities::session::api::AgentMeta;
use crate::capabilities::session::api::MemberTools;
use crate::capabilities::session::api::{CheckView, LineView, Pending, SessionEvent};
use crate::capabilities::workspace::api::Module;
use std::sync::Arc;

impl CollabSession {
    /// 泵：推进讨论直至暂停（ask）或收敛并走完整理/执行/验收/交付；事件逐条经 sink 外送。
    pub fn pump_with(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_none() {
            return;
        }
        let prompts = self.prompts.clone();
        // 讨论阶段：只在未收敛时步进（回档/重启后可从中途接着走）。
        if !self.disc.as_ref().expect("disc 已确认存在").closed {
            loop {
                // 已在等用户（请教 / 待审 / 待继续）：泵不再往下推——驱动循环据此停下。
                if self.awaiting_user() {
                    return;
                }
                // 逐成员外送：一个成员说完就出它那一行（整轮问完才一次性出会让界面整轮不动）。
                // 回调里不能借 self（disc 正被可变借用），所以用 Cell/RefCell 暂存，调用后并回会话。
                // 上一个成员回合失败 / 被停：如实交回（不静默吞掉，也不当发言吸收）。
                let outcome = if let Some(err) = self.turn_error.take() {
                    TurnOut::Interrupted(err)
                } else {
                    // 泵只推**一步**：该问谁就存下并让出——驱动权在核心（它同时看得到协作会话与各 agent 的会话）。
                    match self.disc.as_mut().expect("disc 已确认存在").advance() {
                        crate::capabilities::collab::service::discussion::Adv::Ask {
                            i,
                            identity,
                            turn,
                        } => {
                            self.pending_ask = Some((i, identity, turn));
                            return;
                        }
                        // 开场刚问完：接着进轮次。
                        crate::capabilities::collab::service::discussion::Adv::Opened => continue,
                        crate::capabilities::collab::service::discussion::Adv::Out(out) => out,
                    }
                };
                match outcome {
                    TurnOut::Round => {}
                    TurnOut::Interrupted(err) => {
                        // 讨论中调用失败 / 被停：**不**把它当发言吸收，如实告知并中断本轮。
                        // 停止与失败用不同文案（用户看得到"是我停的"还是"它断了"）。
                        let note = if self.cancelled() {
                            crate::capabilities::session::api::stopped_note()
                        } else {
                            crate::capabilities::session::api::interrupted_note(&err)
                        };
                        sink(SessionEvent::Notice(note));
                        return;
                    }
                    TurnOut::Stopped => {
                        // 用户点了「停止」：被中断的那条发言没有吸收（半截 say/agree 会把状态算歪），
                        // 讨论保持可继续——点「继续」从断点接着推进。
                        sink(SessionEvent::Notice(
                            crate::capabilities::session::api::stopped_note(),
                        ));
                        return;
                    }
                    TurnOut::AskUser { member, question } => {
                        self.ask_user(Pending::Ask { member, question }, sink);
                        return;
                    }
                    TurnOut::Done => {
                        let round = self.disc.as_ref().expect("disc 存在").round;
                        let over_cap = round > MAX_ROUNDS;
                        if over_cap {
                            sink(SessionEvent::Notice(
                                "[上限] 讨论轮次超限，交用户裁决。".into(),
                            ));
                        }
                        sink(SessionEvent::DiscussionDone { round, over_cap });
                        break;
                    }
                }
            }
        }
        // 整理：只在还没有方案（或没有链）时做——回档/重启后沿用已记的，不重复花钱。
        if self.plan.is_none() || self.chain.is_none() {
            sink(crate::capabilities::session::api::working("核心"));
            let mut verify = self.core_verify_tools("planner");
            let made = self.disc.as_ref().expect("disc 存在").synthesize(
                self.core_chat.as_mut(),
                self.core_mode,
                verify.as_mut(),
                sink,
            );
            sink(crate::capabilities::session::api::idle());
            match made {
                Ok((plan, chain, advice)) => {
                    // 核心 AI 的建议随方案一起来（同一批产出，不额外花一次调用）。
                    self.gate_advice = advice;
                    // **装配期门禁**：链必须自洽（悬空依赖 / 环 / 未知负责人 / 空目标）——
                    // 不静默开工；挡下时如实说明，用户点「继续」会重新整理。
                    let roster: Vec<String> = self
                        .disc
                        .as_ref()
                        .expect("disc 存在")
                        .members
                        .iter()
                        .filter(|m| m.present)
                        .map(|m| m.id.clone())
                        .collect();
                    let problems = chain.problems(&roster);
                    if !problems.is_empty() {
                        sink(SessionEvent::Notice(format!(
                            "[错误] 核心给出的任务链不自洽：{}。点「继续」会重新整理。",
                            problems.join("；")
                        )));
                        return;
                    }
                    // **按阶段定名**：阶段由依赖图派生，节点 id 由核心给（n{阶段}-{序号}），
                    // 模型给的 id 只用来解析依赖——用户看到的序号因此带前后关系。
                    let mut chain = chain;
                    chain.renumber_by_stage();
                    self.plan = Some(plan.clone());
                    self.chain = Some(chain);
                    sink(SessionEvent::Plan(plan));
                }
                // 整理被停止 / 失败 / 回执不合法：都不落方案、不往下走，如实告知并交回用户。
                Err(err) => {
                    let note = if self.cancelled() {
                        crate::capabilities::session::api::stopped_note()
                    } else {
                        crate::capabilities::session::api::interrupted_note(&err)
                    };
                    sink(SessionEvent::Notice(note));
                    return;
                }
            }
        }
        let plan = self.plan.clone().unwrap_or_default();
        // **审查关卡**：整理完不自动开工——方案与链先交用户审查，点「同意」才推进。
        // 为什么闸门在这里：整理之后就是花钱的执行（每个成员一轮工具循环），让用户先看一眼最省事。
        if !self.plan_approved {
            sink(SessionEvent::PlanReview {
                plan: plan.clone(),
                chain: self.chain.clone().unwrap_or_default(),
            });
            self.ask_user(Pending::PlanReview, sink);
            return;
        }
        // **一提交就报完成**：节点 Done 但还没验收的那一步就是"完成"这一条（不是攒到总验收才报）。
        // 用户看到的顺序因此是：谁 开工 → 谁 节点完成 → 阶段通过 / 返工。
        let freshly: Vec<(String, String)> = self
            .chain
            .as_ref()
            .map(|c| {
                c.nodes
                    .iter()
                    .filter(|n| {
                        n.status == crate::capabilities::taskchain::api::NodeStatus::Done
                            && !n.reported
                    })
                    .map(|n| {
                        (
                            n.id.clone(),
                            n.report
                                .clone()
                                .unwrap_or_else(|| "（该节点没有产出）".to_string()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (id, text) in freshly {
            if let Some(c) = self.chain.as_mut() {
                if let Some(n) = c.nodes.iter_mut().find(|n| n.id == id) {
                    n.reported = true;
                }
            }
            sink(SessionEvent::Report {
                id,
                text,
                rework: 0,
            });
        }
        // 等用户的事挂着（请教 / 方案待审 / 节点没过）：**泵不往下推**——唤醒（子会话完成、
        // 别的客户端动作）也不能替用户点「继续」，否则"暂停"形同虚设（真机上演过：总验收没过、
        // 本该停下等用户，节点子会话一完成就把那些节点又派了一遍）。重派在**用户那一步**做（见 resume）。
        if self.awaiting_user() {
            sink(crate::capabilities::session::api::idle());
            return;
        }
        // **阶段驱动**：同一阶段（依赖图里同一层）的节点并发跑，跨阶段串行。
        // 本阶段跑完 → 核心 AI 做**一次阶段验收**（判这一阶段的产出够不够下一阶段用）；
        // 通过才解锁下一阶段；没过就暂停交用户——**重派哪些节点由核心的结论决定**
        // （结论里没通过的才退回待办，同阶段其余节点保持已通过，不整阶段重来）。
        while let Some(stage) = self.chain.as_ref().and_then(|c| c.current_stage()) {
            // 本阶段还有节点在跑 / 待派：让出（核心侧派发）。
            if !self
                .chain
                .as_ref()
                .map(|c| c.stage_settled(stage))
                .unwrap_or(false)
            {
                // 节点跑在各自的子会话里：主会话这一刻没有"谁在干活"，
                // 但**子会话在跑**要照实显示（前端按运行态快照把主会话标成在跑）。
                sink(crate::capabilities::session::api::idle());
                return;
            }
            // 这一阶段的节点逐个判（核心 AI 给结论，也由它决定重派哪些）。
            let stage_nodes: Vec<crate::capabilities::taskchain::api::TaskNode> = self
                .chain
                .as_ref()
                .map(|c| c.stage_nodes(stage).into_iter().cloned().collect())
                .unwrap_or_default();
            let known: Vec<String> = stage_nodes.iter().map(|n| n.id.clone()).collect();
            // **填错就一直重填**（不设次数上限；用户用「停止」控制流程）：判定必须落到这一阶段的
            // 节点上，否则"退回待办并重派"的名单就是错的。
            let mut retry: Option<String> = None;
            let (verdicts, advice) = loop {
                let reviewed = crate::capabilities::taskchain::api::TaskChain {
                    nodes: stage_nodes.clone(),
                };
                sink(crate::capabilities::session::api::working("核心"));
                let mut verify = self.core_verify_tools("orchestrator");
                let made = Self::review_nodes(
                    &*self.prompts,
                    &*self.systools,
                    &self.cancel,
                    Some(&reviewed),
                    crate::capabilities::llm::api::CompleteOpts::plain(self.settings.app.streaming)
                        .with_timeout(self.settings.app.llm_timeout_secs),
                    self.core_mode,
                    self.core_chat.as_mut(),
                    verify.as_mut(),
                    retry.as_deref(),
                    sink,
                );
                sink(crate::capabilities::session::api::idle());
                let (verdicts, advice) = match made {
                    Ok(v) => v,
                    Err(err) => {
                        sink(SessionEvent::Notice(
                            crate::capabilities::session::api::interrupted_note(&err),
                        ));
                        return;
                    }
                };
                let unknown: Vec<String> = verdicts
                    .iter()
                    .map(|(n, _, _)| n.clone())
                    .filter(|n| !known.iter().any(|k| k == n))
                    .collect();
                let missing: Vec<String> = known
                    .iter()
                    .filter(|k| !verdicts.iter().any(|(n, _, _)| n == *k))
                    .cloned()
                    .collect();
                if unknown.is_empty() && missing.is_empty() {
                    break (verdicts, advice);
                }
                let mut what: Vec<String> = Vec::new();
                if !unknown.is_empty() {
                    what.push(format!("不在表里的 id：{}", unknown.join("、")));
                }
                if !missing.is_empty() {
                    what.push(format!("没给结论的节点：{}", missing.join("、")));
                }
                sink(SessionEvent::Notice(format!(
                    "[阶段 {} 验收] 这次判定用不了（{}），已要求核心重填；要停就点「停止」。",
                    stage,
                    what.join("；")
                )));
                if self.cancelled() {
                    sink(SessionEvent::Notice(
                        crate::capabilities::session::api::stopped_note(),
                    ));
                    return;
                }
                retry = Some(format!(
                    "你上一次的判定没落到这一阶段的节点上（{}）。请只从下面这张表里选 node，并且**每个节点都给一条结论**：\n{}",
                    what.join("；"),
                    stage_nodes
                        .iter()
                        .map(|n| format!("- {} — 负责人 {}", n.id, n.assignee))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            };
            // 核心 AI 的建议随验收结论一起来（同一批产出）。
            self.gate_advice = advice;
            for (node, ok, note) in &verdicts {
                self.set_node_acceptance(node, *ok, note);
            }
            let bad: Vec<String> = verdicts
                .iter()
                .filter(|(_, ok, _)| !ok)
                .map(|(n, _, _)| n.clone())
                .collect();
            if !bad.is_empty() {
                // 每个没过节点单列一条**返工提示**（带核心给的原因），用户一眼看到要返工谁、差在哪。
                for (node, ok, note) in &verdicts {
                    if *ok {
                        continue;
                    }
                    let why = note.trim();
                    sink(SessionEvent::Notice(if why.is_empty() {
                        format!("[返工] {}：没过（等用户点「继续」后只重派它）", node)
                    } else {
                        format!("[返工] {}：{}", node, why)
                    }));
                }
                sink(SessionEvent::Notice(format!(
                    "[阶段 {} 验收] 没通过：{}。点「继续」后**只重派这些**（下一阶段先不开工）。",
                    stage,
                    bad.join("、")
                )));
                self.ask_user(Pending::NodeBlocked { nodes: bad }, sink);
                return;
            }
            sink(SessionEvent::Notice(format!(
                "[阶段 {} 通过] 下一阶段开工。",
                stage
            )));
        }
        // 总验收：核心 AI 按**各节点的产出**核对（复用执行阶段的验收机制）→ 交付。
        let llm = self.llm_opts();
        let mut exec = Execution::new();
        for n in self.chain.as_ref().expect("链存在").nodes.iter() {
            let note = n
                .report
                .clone()
                .unwrap_or_else(|| "（该节点没有产出）".to_string());
            // 只把回报交给验收用；"[节点] 完成"那条在**节点提交那一刻**就报过了（见泵的"一提交就报完成"）。
            exec.reports.insert(n.id.clone(), note);
        }
        // "节点 id — 负责人"对照表：模型只能从它里面选 rework。
        let table = self
            .chain
            .as_ref()
            .map(|c| {
                c.nodes
                    .iter()
                    .map(|n| format!("- {} — 负责人 {}", n.id, n.assignee))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let known: Vec<String> = self
            .chain
            .as_ref()
            .map(|c| c.nodes.iter().map(|n| n.id.clone()).collect())
            .unwrap_or_default();
        // **填错就一直重填**（不设次数上限；用户用「停止」控制）：没过（fail）的条目必须指名
        // 要返工的节点，且只能取上面那张表里的 id——退错了节点等于让错的人白跑一遍。
        let mut retry: Option<String> = None;
        loop {
            sink(crate::capabilities::session::api::working("核心"));
            let mut verify = self.core_verify_tools("orchestrator");
            exec.review(
                self.core_chat.as_mut(),
                &plan,
                &table,
                retry.as_deref(),
                &*prompts,
                &*self.systools,
                llm,
                self.core_mode,
                verify.as_mut(),
                sink,
            );
            sink(crate::capabilities::session::api::idle());
            if let Some(note) = self.exec_note(&exec) {
                sink(SessionEvent::Notice(note));
                return;
            }
            let problems = exec.rework_problems(&known);
            if problems.is_empty() {
                break;
            }
            sink(SessionEvent::Notice(format!(
                "[总验收] 这次判定用不了（{}），已要求核心重填；要停就点「停止」。",
                problems.join("；")
            )));
            if self.cancelled() {
                sink(SessionEvent::Notice(
                    crate::capabilities::session::api::stopped_note(),
                ));
                return;
            }
            retry = Some(format!(
                "你上一次的清单用不了（{}）。没过（fail）的条目**必须**填 rework，且只能取下面这张表里的节点 id：\n{}",
                problems.join("；"),
                table
            ));
        }
        sink(review_event(&exec));
        // 没过 = 只把这些节点退回待办，等用户点「继续」后重派（不交付）。
        let bad = exec.rework_targets();
        if !bad.is_empty() {
            // 每个要返工的节点单列一条（带核心给的原因）：用户一眼看到返工谁、差在哪。
            for it in &exec.items {
                if !it.status.eq_ignore_ascii_case("fail") {
                    continue;
                }
                let node = it.rework.as_deref().unwrap_or("");
                let why = it.reason.as_deref().unwrap_or("").trim();
                sink(SessionEvent::Notice(if why.is_empty() {
                    format!("[返工] {}：没过（等用户点「继续」后只重派它）", node)
                } else {
                    format!("[返工] {}：{}", node, why)
                }));
            }
            sink(SessionEvent::Notice(format!(
                "[总验收] 没通过：{}。点「继续」后**只重派这些**（不交付）。",
                bad.join("、")
            )));
            self.ask_user(Pending::NodeBlocked { nodes: bad }, sink);
            return;
        }
        sink(SessionEvent::Delivery {
            ok: exec.all_pass(),
            over_rework: false,
        });
        sink(SessionEvent::Ended);
        self.done = true;
    }

    /// 从在组名单装配成员通道（带回落告知；策略在 conductor：该 agent 的模型 > 核心默认）。
    /// 一个 agent = 一个成员：system 由它全部模块合成，工具 = 各模块外部工具的并集。
    pub(crate) fn assemble_members(&self) -> Result<(Vec<Member>, Vec<String>), String> {
        let prompts = self.prompts.clone();
        let roster = self.workspace.roster();
        let library = self.workspace.library();
        let mut members = Vec::new();
        let mut notes = Vec::new();
        for a in &self.roster {
            // 该 agent 的模块：清单即事实，缺了就如实报错（不静默跳过）。
            let modules: Vec<Module> = a
                .modules
                .iter()
                .filter_map(|id| {
                    roster
                        .modules
                        .iter()
                        .find(|m| &m.manifest.id == id)
                        .cloned()
                })
                .collect();
            if modules.len() != a.modules.len() {
                let missing: Vec<String> = a
                    .modules
                    .iter()
                    .filter(|id| !roster.modules.iter().any(|m| &&m.manifest.id == id))
                    .cloned()
                    .collect();
                return Err(format!(
                    "agent {} 的模块已不在清单：{}",
                    a.name,
                    missing.join("、")
                ));
            }
            let channel = a
                .model
                .as_deref()
                .and_then(|id| self.settings.resolve(id).ok())
                .or_else(|| self.settings.core_channel());
            // 通道本身不再由成员持有（回合跑在各自的 agent 会话里）；这里只取它的如实告知。
            let (_chat, note) = self.llm.member_channel(channel.as_ref(), &a.name);
            if let Some(n) = note {
                notes.push(n);
            }
            let sandbox = self.sandboxes.for_agent(&a.name).cloned().ok_or_else(|| {
                format!("agent {} 没有被分配沙箱（工作区未记录该 agent）", a.name)
            })?;
            // 形态按该 agent 的模型（或核心默认）解析：身份块里的调用约定与实际协议必须一致
            let mode = if channel.is_some() {
                self.settings.tool_mode_for(a.model.as_deref())
            } else {
                crate::capabilities::llm::api::ToolMode::Envelope
            };
            // **会话参数**：身份块每回合由它现渲染，不存进任何人的消息列表。
            let params = crate::capabilities::session::api::SessionParams::from_workspace(
                &a.name,
                &sandbox,
                &modules,
                vec![crate::capabilities::prompt::api::Segment::MechanismCollab],
            );
            let mut member = Member::plain(&a.name, params, mode);
            // 围栏：可达范围 + 断网，由该 agent 的沙箱与 exec 段派生（机制在 adapters）；
            // 只读根来自用户显式授权（`fence_read`），默认空。
            let fence =
                crate::capabilities::tools::api::FenceSpec::from_sandbox(&sandbox, self.spec.net)
                    .with_read_only(read_only_roots(&self.settings.app));
            // 工具说明块的素材（patch 语法 / 模块工具 / 模块参数）：装配期按这个 agent 的沙箱与模块算一次。
            let tool_notes =
                crate::capabilities::tools::api::tool_notes(&*prompts, &sandbox, &modules);
            member.tools = Some(MemberTools {
                mode,
                // 模块 id → 该模块的（目录, 工具表）：多模块 agent 靠信封里的 module 消歧。
                modules: crate::capabilities::session::api::tool_table(&modules),
                observations: crate::capabilities::tools::api::Observations::default(),
                llm: Arc::clone(&self.llm),
                log: Arc::clone(&self.log),
                tools: Arc::clone(&self.tools),
                sandbox,
                builtin_tools: self.systools.book(),
                reply_seq: self.reply_seq,
                // 本档位下不能执行工具的模块（缺运行包）：机制侧据此拒绝执行。
                unavailable: crate::capabilities::workspace::api::unavailable(
                    &self.spec, &modules, &library,
                ),
                fence,
                // 讨论席的系统工具面**由角色表发放**（越权校验的唯一判据）。
                allowed: self
                    .systools
                    .tool_face("discussant")
                    .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
                    .unwrap_or_default(),
                // 讨论席不干活：拿不到自己模块的工具（角色表的 module_tools）。
                with_modules: self.systools.allows_module_tools("discussant"),
                notes: tool_notes,
                handlers: Vec::new(),
            });
            members.push(member);
        }
        Ok((members, notes))
    }
}

/// 代拟行里的一项（只给人看）：复用项标出来，组装项带上模块与模型。
pub(crate) fn slate_item(a: &AgentMeta, why: &str) -> String {
    if a.transient {
        format!(
            "{}〈{}〉→ {}（{}）",
            a.name,
            a.modules.join(","),
            a.model.clone().unwrap_or_default(),
            why
        )
    } else {
        format!("{}（复用；{}）", a.name, why)
    }
}

/// 从派生状态推出当前挂起（None = 没有待用户处理的门）。
pub(crate) fn derive_pending(
    st: &crate::capabilities::collab::domain::collab_state::CollabState,
) -> Option<Pending> {
    if st.ended {
        return None;
    }
    if !st.begun {
        if st.slate.is_some() && !st.slate_confirmed {
            return Some(Pending::ConfirmSlate);
        }
        if st.task.is_some() {
            return Some(Pending::ConfirmBegin);
        }
        return None;
    }
    st.pending_ask.as_ref().map(|(m, q)| Pending::Ask {
        member: m.clone(),
        question: q.clone(),
    })
}

/// 逐成员外送：把刚定稿的讨论行变成带**会话内稳定 id** 的转录事件交出去。
/// 为什么要 Cell/RefCell：回调在 `Discussion::step/open` 内部被调用，那时 `self` 正被可变借用，
/// 碰不到 `self.next_line` 与 `sink`——所以调用前后各并回一次，行只构造一次。
pub(crate) fn emit_new_lines(
    lines: &[LineView],
    next_line: &std::cell::Cell<u64>,
    handed: &std::cell::Cell<usize>,
    sink: &mut dyn FnMut(SessionEvent),
) {
    let views: Vec<LineView> = lines
        .iter()
        .map(|l| {
            // 会话内稳定 id 由**主会话**在行的第一次见光时分配（行自己不带 id）。
            let id = next_line.get();
            next_line.set(id + 1);
            LineView {
                id,
                reply: id,
                ..l.clone()
            }
        })
        .collect();
    handed.set(handed.get() + views.len());
    if !views.is_empty() {
        sink(SessionEvent::Transcript(views));
    }
}

/// 发出自上次以来的新转录行（增量），逐行分配会话内稳定 id。
pub(crate) fn push_delta(
    disc: &Discussion,
    emitted: &mut usize,
    next_line: &mut u64,
    sink: &mut dyn FnMut(SessionEvent),
) {
    if disc.transcript.len() > *emitted {
        let views: Vec<LineView> = disc.transcript[*emitted..]
            .iter()
            .map(|l| {
                // 整行照搬（工具视图 / 降级 / 回合号 / 系统标记都不丢）。
                let v = LineView {
                    id: *next_line,
                    reply: *next_line,
                    ..l.clone()
                };
                *next_line += 1;
                v
            })
            .collect();
        *emitted = disc.transcript.len();
        sink(SessionEvent::Transcript(views));
    }
}

pub(crate) fn review_event(exec: &Execution) -> SessionEvent {
    let items = exec
        .items
        .iter()
        .map(|i| CheckView {
            item: i.item.clone(),
            status: i.status.clone(),
            note: i
                .reason
                .clone()
                .or_else(|| i.evidence.clone())
                .unwrap_or_default(),
        })
        .collect();
    SessionEvent::Review {
        items,
        raw: exec.checklist_raw.clone(),
    }
}
