//! 目的：流程泵——按相位推进（讨论段 pump_discuss、执行段 pump_execute）并装配成员（assemble_members）。
//! 管：该问谁与上下文装配；讨论收敛后由核心代理写总结（summarize）；行外送与增量（emit_new_lines / push_delta / review_event / derive_gate）。
//! 不管：模型调用（泵不自己调模型，把「该问谁、带什么上下文」交回协调业务）；状态机步骤的判定（discussion.rs）。
//! 联动：由 CollabSession 驱动；两段由 `session::domain::sequence::Sequence` 串起来；约定见 docs/session/session-model.md。

use super::collab::{CollabSession, Phase};
use super::lines::*;
use crate::units::catalog::modules::api::Module;
use crate::units::conversation::collab::domain::events::CollabEvent;
use crate::units::conversation::collab::domain::gates::Pending;
use crate::units::conversation::collab::service::discussion::{MAX_ROUNDS, Member, TurnOut};
use crate::units::conversation::collab::service::synthesis::{Execution, draft_plan};
use crate::units::llm::api::Msg;
use crate::units::prompt::api::Segment;
use crate::units::transcript::api::{LineView, SessionEvent};

impl CollabSession {
    /// 目的：泵：按本段流程的相位推进（讨论段收敛后写总结；执行段拟计划、派活、验收、交付）。
    pub fn pump_with(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        match self.phase {
            Phase::Discuss => self.pump_discuss(sink),
            Phase::Execute => self.pump_execute(sink),
        }
    }

    /// 目的：讨论段：推进讨论直至让出一步或收敛；收敛后由核心代理把讨论上下文收成一份总结。
    fn pump_discuss(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_none() {
            return;
        }
        // 队列里还挂着等用户答的卡：泵**一步都不推**（谁也不能替用户答，见 session-model.md）。
        if self.awaiting_user() {
            return;
        }
        // 讨论阶段：只在未收敛时步进（回档/重启后可从中途接着走）。
        if !self.disc.as_ref().expect("disc 已确认存在").closed {
            loop {
                // 已在等用户（请教 / 待审 / 待继续）：泵不再往下推——驱动循环据此停下。
                if self.awaiting_user() {
                    return;
                }
                // 逐成员外送：一个成员说完就出它那一行（整轮问完才一次性出会让界面整轮不动）；回调里不能借 self
                //（disc 正被可变借用），所以用 Cell/RefCell 暂存，调用后并回会话；回合失败 / 被停也在这里如实交回。
                let outcome = if let Some(err) = self.turn_error.take() {
                    TurnOut::Interrupted(err)
                } else {
                    // 泵只推**一步**：该问谁就存下并让出——驱动权在核心（它同时看得到协作会话与各 agent 的会话）。
                    match self.disc.as_mut().expect("disc 已确认存在").advance() {
                        crate::units::conversation::collab::service::discussion::Adv::Ask {
                            i,
                            identity,
                            turn,
                        } => {
                            self.pending_ask = Some((i, identity, turn));
                            return;
                        }
                        // 开场刚问完：接着进轮次。
                        crate::units::conversation::collab::service::discussion::Adv::Opened => {
                            continue;
                        }
                        crate::units::conversation::collab::service::discussion::Adv::Out(out) => {
                            out
                        }
                    }
                };
                match outcome {
                    TurnOut::Round => {}
                    TurnOut::Interrupted(err) => {
                        // 讨论中调用失败 / 被停：**不**把它当发言吸收，如实告知并中断本轮。
                        // 停止与失败用不同文案（用户看得到"是我停的"还是"它断了"）。
                        let note = if self.cancelled() {
                            crate::units::transcript::api::stopped_note()
                        } else {
                            crate::units::transcript::api::interrupted_note(&err)
                        };
                        self.notice(&note, sink);
                        return;
                    }
                    TurnOut::Stopped => {
                        // 用户点了「停止」：被中断的那条发言没有吸收（半截 say/agree 会把状态算歪），
                        // 讨论保持可继续——点「继续」从断点接着推进。
                        self.notice(&crate::units::transcript::api::stopped_note(), sink);
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
                            self.notice("[上限] 讨论轮次超限，交用户裁决。", sink);
                        }
                        sink(CollabEvent::DiscussionDone { round, over_cap }.push());
                        break;
                    }
                }
            }
        }
        // 收敛之后：这一段交出的是**一份需求说明**——由核心代理（不是成员）把讨论上下文收成它，
        // 交给下一步（执行流程）当输入；它同时落成一条转录行，回档/重启后按它派生。
        if self.summary.is_some() {
            self.done = true;
            return;
        }
        sink(crate::units::transcript::api::working("核心"));
        let made = self.summarize(sink);
        sink(crate::units::transcript::api::idle());
        match made {
            Ok(text) => {
                let line = self.view(LineView::system("总结", text.clone()));
                sink(SessionEvent::Transcript(vec![line]));
                sink(CollabEvent::Summary(text.clone()).push());
                self.summary = Some(text);
                self.done = true;
            }
            Err(err) => {
                let note = if self.cancelled() {
                    crate::units::transcript::api::stopped_note()
                } else {
                    crate::units::transcript::api::interrupted_note(&err)
                };
                self.notice(&note, sink);
            }
        }
    }

    /// 目的：执行段：从交来的需求起步，拟计划、等审批、派节点、阶段验收、总验收与交付。
    fn pump_execute(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done {
            return;
        }
        // 队列里还挂着等用户答的卡：一步都不推（与讨论段同一条规则）。
        if self.awaiting_user() {
            return;
        }
        if self.task.trim().is_empty() {
            self.notice(
                "[错误] 执行这一步没有拿到要做的需求：先交一份讨论总结或需求说明。",
                sink,
            );
            return;
        }
        let prompts = self.prompts.clone();
        // 拟计划：只在还没有方案（或没有链）时做——回档/重启后沿用已记的，不重复花钱。
        if self.plan.is_none() || self.chain.is_none() {
            sink(crate::units::transcript::api::working("核心"));
            // 交给核心代理的输入就是**交来的那份需求说明**（讨论的总结）：讨论与执行的对接只有这一条消息。
            let roster = self.names().join("、");
            let task = self.task.clone();
            let opts = self.disc_opts();
            let cancel = std::sync::Arc::clone(&self.cancel);
            let made = {
                let mut verify_box = self.verify_env("planner");
                let verify = verify_box.as_deref_mut();
                draft_plan(
                    &*self.prompts,
                    &*self.systools,
                    &roster,
                    &task,
                    self.core_chat.as_mut(),
                    self.core_mode,
                    opts,
                    &cancel,
                    verify,
                    sink,
                )
            };
            sink(crate::units::transcript::api::idle());
            match made {
                Ok((plan, chain, advice)) => {
                    // 核心 AI 的建议随方案一起来（同一批产出，不额外花一次调用）。
                    self.gate_advice = advice;
                    // **装配期门禁**：链必须自洽（悬空依赖 / 环 / 未知负责人 / 空目标），不静默开工；
                    // 挡下时如实说明，用户点「继续」会重新整理。能接活的是**在组名单**（名单就是它）。
                    let roster = self.names();
                    let problems = chain.problems(&roster);
                    if !problems.is_empty() {
                        self.notice(
                            &format!(
                                "[错误] 核心给出的任务链不自洽：{}。点「继续」会重新整理。",
                                problems.join("；")
                            ),
                            sink,
                        );
                        return;
                    }
                    // **按阶段定名**：阶段由依赖图派生，节点 id 由核心给（n{阶段}-{序号}），
                    // 模型给的 id 只用来解析依赖——用户看到的序号因此带前后关系。
                    let mut chain = chain;
                    chain.renumber_by_stage();
                    self.plan = Some(plan.clone());
                    self.chain = Some(chain);
                    sink(CollabEvent::Plan(plan.clone()).push());
                    // 方案正文同次落一条转录行：刷新与回档后照样读得到（事件只作增量，流程视图要用）。
                    self.record("核心整理", &plan, sink);
                }
                // 整理被停止 / 失败 / 回执不合法：都不落方案、不往下走，如实告知并交回用户。
                Err(err) => {
                    let note = if self.cancelled() {
                        crate::units::transcript::api::stopped_note()
                    } else {
                        crate::units::transcript::api::interrupted_note(&err)
                    };
                    self.notice(&note, sink);
                    return;
                }
            }
        }
        let plan = self.plan.clone().unwrap_or_default();
        // **审查关卡**：整理完不自动开工——方案与链先交用户审查，点「同意」才推进。
        // 为什么闸门在这里：整理之后就是花钱的执行（每个成员一轮工具循环），让用户先看一眼最省事。
        if !self.plan_approved {
            sink(
                CollabEvent::PlanReview {
                    plan: plan.clone(),
                    chain: self.chain.clone().unwrap_or_default(),
                }
                .push(),
            );
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
                        n.status == crate::units::pure::taskchain::api::NodeStatus::Done
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
            if let Some(c) = self.chain.as_mut()
                && let Some(n) = c.nodes.iter_mut().find(|n| n.id == id)
            {
                n.reported = true;
            }
            sink(
                CollabEvent::Report {
                    id,
                    text,
                    rework: 0,
                }
                .push(),
            );
        }
        // 等用户的事挂着（请教 / 方案待审 / 节点没过）：**泵不往下推**——唤醒（子会话完成、别的客户端动作）
        // 也不能替用户点「继续」，否则"暂停"形同虚设；重派在**用户那一步**做（见 resume）。
        if self.awaiting_user() {
            sink(crate::units::transcript::api::idle());
            return;
        }
        // **阶段驱动**：同一阶段（依赖图里同一层）的节点并发跑，跨阶段串行；本阶段跑完 → 核心 AI 做**一次阶段验收**。
        // 通过才解锁下一阶段；没过就暂停交用户——**重派哪些节点由核心的结论决定**（退待办，其余保持已通过）。
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
                sink(crate::units::transcript::api::idle());
                return;
            }
            // 这一阶段的节点逐个判（核心 AI 给结论，也由它决定重派哪些）。
            let stage_nodes: Vec<crate::units::pure::taskchain::api::TaskNode> = self
                .chain
                .as_ref()
                .map(|c| c.stage_nodes(stage).into_iter().cloned().collect())
                .unwrap_or_default();
            let known: Vec<String> = stage_nodes.iter().map(|n| n.id.clone()).collect();
            // **填错就一直重填**（不设次数上限；用户用「停止」控制流程）：判定必须落到这一阶段的
            // 节点上，否则"退回待办并重派"的名单就是错的。
            let mut retry: Option<String> = None;
            let (verdicts, advice) = loop {
                let reviewed = crate::units::pure::taskchain::api::TaskChain {
                    nodes: stage_nodes.clone(),
                };
                sink(crate::units::transcript::api::working("核心"));
                let made = {
                    let mut verify_box = self.verify_env("orchestrator");
                    let verify = verify_box.as_deref_mut();
                    Self::review_nodes(
                        &*self.prompts,
                        &*self.systools,
                        &self.cancel,
                        Some(&reviewed),
                        crate::units::llm::api::CompleteOpts::plain(self.settings.app.streaming)
                            .with_timeout(self.settings.app.llm_timeout_secs),
                        self.core_mode,
                        self.core_chat.as_mut(),
                        verify,
                        retry.as_deref(),
                        sink,
                    )
                };
                sink(crate::units::transcript::api::idle());
                let (verdicts, advice) = match made {
                    Ok(v) => v,
                    Err(err) => {
                        self.notice(&crate::units::transcript::api::interrupted_note(&err), sink);
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
                self.notice(
                    &format!(
                        "[阶段 {} 验收] 这次判定用不了（{}），已要求核心重填；要停就点「停止」。",
                        stage,
                        what.join("；")
                    ),
                    sink,
                );
                if self.cancelled() {
                    self.notice(&crate::units::transcript::api::stopped_note(), sink);
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
                    let note = if why.is_empty() {
                        format!("[返工] {}：没过（等用户点「继续」后只重派它）", node)
                    } else {
                        format!("[返工] {}：{}", node, why)
                    };
                    self.notice(&note, sink);
                }
                self.notice(
                    &format!(
                        "[阶段 {} 验收] 没通过：{}。点「继续」后**只重派这些**（下一阶段先不开工）。",
                        stage,
                        bad.join("、")
                    ),
                    sink,
                );
                self.ask_user(Pending::NodeBlocked { nodes: bad }, sink);
                return;
            }
            self.notice(&format!("[阶段 {} 通过] 下一阶段开工。", stage), sink);
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
            sink(crate::units::transcript::api::working("核心"));
            {
                let mut verify_box = self.verify_env("orchestrator");
                let verify = verify_box.as_deref_mut();
                exec.review(
                    self.core_chat.as_mut(),
                    &plan,
                    &table,
                    retry.as_deref(),
                    &*prompts,
                    &*self.systools,
                    llm.clone(),
                    self.core_mode,
                    verify,
                    sink,
                )
            };
            sink(crate::units::transcript::api::idle());
            let failed = self.exec_note(&exec);
            if let Some(note) = failed {
                self.notice(&note, sink);
                return;
            }
            let problems = exec.rework_problems(&known);
            if problems.is_empty() {
                break;
            }
            self.notice(
                &format!(
                    "[总验收] 这次判定用不了（{}），已要求核心重填；要停就点「停止」。",
                    problems.join("；")
                ),
                sink,
            );
            if self.cancelled() {
                self.notice(&crate::units::transcript::api::stopped_note(), sink);
                return;
            }
            retry = Some(format!(
                "你上一次的清单用不了（{}）。没过（fail）的条目**必须**填 rework，且只能取下面这张表里的节点 id：\n{}",
                problems.join("；"),
                table
            ));
        }
        // 清单同样先落行再推事件：行给人看（Markdown 一条一行），事件给流程视图与状态派生。
        self.record("验收清单", &review_text(&exec), sink);
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
                let note = if why.is_empty() {
                    format!("[返工] {}：没过（等用户点「继续」后只重派它）", node)
                } else {
                    format!("[返工] {}：{}", node, why)
                };
                self.notice(&note, sink);
            }
            self.notice(
                &format!(
                    "[总验收] 没通过：{}。点「继续」后**只重派这些**（不交付）。",
                    bad.join("、")
                ),
                sink,
            );
            self.ask_user(Pending::NodeBlocked { nodes: bad }, sink);
            return;
        }
        let ok = exec.all_pass();
        // 交付结论也落行：这是"这次协作怎么收的"的事实，不能只靠推的那条事件。
        let text = if ok {
            "全部节点通过总验收，交付用户。"
        } else {
            "总验收没全过（清单里有非通过项），交用户裁决。"
        };
        self.record("交付", text, sink);
        sink(
            CollabEvent::Delivery {
                ok,
                over_rework: false,
            }
            .push(),
        );
        sink(SessionEvent::Ended);
        self.done = true;
    }

    /// 目的：核心代理的一次普通生成：把讨论上下文收成一份需求说明（讨论这一步的产物，交给执行流程）。
    /// 错误：被停、调用失败、回复为空都如实报错——不编一份假的总结往下交。
    fn summarize(&mut self, sink: &mut dyn FnMut(SessionEvent)) -> Result<String, String> {
        let transcript = self
            .disc
            .as_ref()
            .map(|d| {
                d.transcript
                    .iter()
                    .map(|l| l.render())
                    .collect::<Vec<String>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let msgs = vec![
            Msg::system(self.prompts.text(Segment::SummarizeSystem).to_string()),
            Msg::user(
                self.prompts
                    .render(Segment::SummarizeUser, &[("transcript", transcript)]),
            ),
        ];
        let mut opts = self.disc_opts();
        opts.cancel = Some(crate::units::llm::api::cancel_flag(std::sync::Arc::clone(
            &self.cancel,
        )));
        let mut acc = String::new();
        let stop = std::sync::Arc::clone(&self.cancel);
        let made = {
            let mut on = |chunk: crate::units::llm::api::Chunk| {
                let mut kind = "text";
                let mut piece = String::new();
                match &chunk {
                    crate::units::llm::api::Chunk::Start => {
                        acc.clear();
                        kind = "start";
                    }
                    crate::units::llm::api::Chunk::Text(t) => {
                        let (send, next) = crate::units::agentloop::api::stream_piece(&acc, t);
                        piece = send;
                        acc = next;
                    }
                    crate::units::llm::api::Chunk::Reasoning(r) => {
                        kind = "reasoning";
                        piece = r.clone();
                    }
                }
                // 空片段不刷事件流（与各回合同一条规则）。
                if crate::units::agentloop::api::delta_visible(kind, &piece) {
                    sink(crate::units::transcript::api::SessionEvent::Delta {
                        speaker: "核心".to_string(),
                        kind: kind.to_string(),
                        text: piece,
                    });
                }
                !stop.load(std::sync::atomic::Ordering::Relaxed)
            };
            self.core_chat.complete(&msgs, opts, &mut on)
        };
        if made.cancelled || self.cancelled() {
            return Err("已停止".to_string());
        }
        if let Some(err) = made.error {
            return Err(err);
        }
        let text = made.raw.trim().to_string();
        if text.is_empty() {
            return Err("核心没有给出总结（回复为空）".to_string());
        }
        Ok(text)
    }

    /// 目的：从在组名单装配成员（策略在 runtime：该 agent 的模型 > 核心默认；缺了就如实报错）。
    ///   一个 agent = 一个成员：system 由它全部模块合成，工具 = 各模块外部工具的并集。
    pub(crate) fn assemble_members(&self) -> Result<Vec<Member>, String> {
        let roster = self.modules.roster();
        let mut members = Vec::new();
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
            // 通道本身不再由成员持有（回合跑在各自的 agent 会话里）；这里只确认它存在。
            // **不吞解析错误**：模型/供应商配坏了要如实报，不能伪装成"没配置"再回落。
            self.settings
                .effective_channel(a.model.as_deref())
                .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
            let sandbox = self.sandboxes.for_agent(&a.name).cloned().ok_or_else(|| {
                format!("agent {} 没有被分配沙箱（工作区未记录该 agent）", a.name)
            })?;
            // 形态按该 agent 的模型（或核心默认）解析：身份块里的调用约定与实际协议必须一致
            let mode = self.settings.tool_mode_for(a.model.as_deref());
            // **会话参数**：身份块每回合由它现渲染，不存进任何人的消息列表。
            let params = crate::units::agentloop::api::SessionParams::from_workspace(
                &a.name,
                &sandbox,
                &modules,
                "discussant",
            )
            .with_chara(a.chara.clone());
            let member = Member::plain(&a.name, params, mode);
            members.push(member);
        }
        Ok(members)
    }
}
