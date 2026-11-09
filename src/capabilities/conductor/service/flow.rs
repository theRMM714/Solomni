//! 目的：协作流水线推进与协作动作——链推进（advance_chain）、节点派发（drive_node / spawn_ready_nodes / spawn_sub_session）、协作步（collab_advance / collab_resume）、裁决卡动作与改需求。
//! 管：取对象、推进、放回（take_* / put_*）；协作的入口动作（写需求、取卡、处置推进、撤回同意、改需求、查挂起）。
//! 不管：单 agent 的回合收发（turn.rs）、会话启用（activation.rs）、拟名单（suggest.rs）。
//! 联动：api 层的协作接口直接转发到这里。
//!   本方法族只碰 sessions / history / running / desk 这几个字段。

use super::*;

impl Conductor {
    /// 目的：**同步**跑一个节点的子会话（CLI 与测试走这条；Web 生产路径由 ConductorHandle 起工作线程）。
    pub(crate) fn drive_node(&mut self, child: &str, objective: &str) -> Vec<SessionEvent> {
        self.bump_turn_of_child(child);
        match self.prepare_single(child, Some(objective), true) {
            Ok(Prepared::Run {
                session,
                prefix,
                llm,
                ..
            }) => {
                let mut session = *session;
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                // 两个出口（流式短暂事件 / 定稿事件）都要收进同一份事件流：用 RefCell 共享。
                let out = std::cell::RefCell::new(prefix);
                {
                    let mut live = crate::capabilities::session::api::Live {
                        llm,
                        cancel,
                        emit: &mut |ev: SessionEvent| out.borrow_mut().push(ev),
                        // 同步驱动、生成期间读不到键盘：这一趟不接工具级确认，也没有可回答的前端。
                        decisions: None,
                        ask: None,
                    };
                    let mut sink = |ev: SessionEvent| out.borrow_mut().push(ev);
                    // 执行提示词是**核心派的活**（派发行）：界面系统行、上下文 user 角色。
                    let identity = session
                        .params()
                        .identity(&*self.prompt, session.tool_mode());
                    crate::capabilities::collab::api::dispatch_task(
                        &mut session,
                        objective,
                        &identity,
                        &mut live,
                        &mut sink,
                    );
                }
                let events = out.into_inner();
                self.put_single_recorded(child, session, &events);
                events
            }
            _ => Vec::new(),
        }
    }

    /// 目的：推进协作（**核心驱动**）：泵只决定"该问谁"，核心取该 agent 的会话跑这一回合再交回。
    ///  契约见 docs/session/session-model.md 二之二。事件由调用方统一落档。
    pub fn collab_advance(&mut self, sid: &str) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        // 安全网计数（见循环尾）：提醒/重问必须有终点，不能让驱动空转。
        let mut guard = 0usize;
        loop {
            // ① 泵推一步：协作会话**裸搬**（不碰任务链派发那套副作用）。
            let (ask, systools, cancel, opts, member, turn_id) = {
                let mut c = self.take_collab_raw(sid)?;
                c.start_if_needed();
                c.pump_with(&mut |e| out.push(e));
                let ask = c.take_ask();
                let member = ask.as_ref().and_then(|(i, _, _)| c.member_id(*i));
                // 角色表按值带出来（小表）：驱动要它来发放工具面与校验越权。
                let systools = c.systools();
                let cancel = c.disc_cancel();
                let opts = c.disc_opts();
                let turn_id = if ask.is_some() { c.next_turn_id() } else { 0 };
                self.sessions.insert(sid.to_string(), Session::Collab(c));
                (ask, systools, cancel, opts, member, turn_id)
            };
            let (Some((i, identity, turn)), Some(agent), turn_id) = (ask, member, turn_id) else {
                break;
            };
            // ② 该 agent 的会话：没有就按需建（名单确认时已建，这里兜底）。
            let child = format!("{}--{}", sid, agent);
            if self.history.load(&child).is_err() {
                self.spawn_agent_session(sid, &agent)?;
            }
            // ③ 跑这一回合：**与单 agent 同一条轮循环**（身份块 + 本回合工具面 + 对话 + 本回合提示 + 表态）。
            // 工具面由角色表发放（讨论席 = 动词 + 只读核实）；产出逐轮落进**它自己的会话**。
            let mut s = self.take_single(&child)?;
            // 思考强度是**工作根**的会话级事实：成员回合也按它解析（成员模型各自映射）。
            let thinking = self.thinking_fragment_for(&child)?;
            s.set_thinking(thinking.clone());
            let round = match self.sessions.get(sid) {
                Some(Session::Collab(c)) => c.round(),
                _ => 0,
            };
            let (ran, notes) = {
                // 两个出口（流式短暂事件 / 定稿事件）都收进同一份事件流。
                let notes = std::cell::RefCell::new(Vec::new());
                let mut live = crate::capabilities::session::api::Live {
                    llm: crate::capabilities::llm::api::LlmOpts {
                        stream: opts.stream,
                        timeout_secs: opts.timeout_secs,
                        thinking,
                    },
                    cancel: std::sync::Arc::clone(&cancel),
                    emit: &mut |e: SessionEvent| notes.borrow_mut().push(e),
                    decisions: None,
                    ask: None,
                };
                let mut sink = |e: SessionEvent| notes.borrow_mut().push(e);
                let face = systools.role_face("discussant");
                let t = crate::capabilities::collab::api::discussion_turn(
                    &mut s, &identity, face, turn, turn_id, round, &mut live, &mut sink,
                );
                (t, notes.into_inner())
            };
            // 落盘到**它自己的目录**（讨论的核实痕迹随会话一起重启后还在）。
            self.persister(&child).persist(&notes);
            out.extend(notes);
            self.put_single(&child, s);
            let turn = match ran {
                Ok(t) => t,
                Err(err) => {
                    let note = if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        crate::capabilities::session::api::stopped_note()
                    } else {
                        crate::capabilities::session::api::interrupted_note(&err)
                    };
                    out.extend(self.notice_row(sid, &note));
                    break;
                }
            };
            // ④ **核心只提醒、不强制**（见 session-model.md 二）：没表态时按计数决定提醒还是放过。
            let user_stopped = cancel.load(std::sync::atomic::Ordering::Relaxed);
            let (after, reminder) = {
                let mut c = self.take_collab_raw(sid)?;
                let a = c.after_member_turn(
                    i,
                    turn.verb.is_some(),
                    user_stopped,
                    self.registry.app().discuss_remind_cap,
                );
                let text = c.reminder_text();
                self.sessions.insert(sid.to_string(), Session::Collab(c));
                (a, text)
            };
            match after {
                AfterTurn::Done => {
                    let mut c = self.take_collab_raw(sid)?;
                    c.feed_with(i, turn, turn_id, &mut |e| out.push(e));
                    self.sessions.insert(sid.to_string(), Session::Collab(c));
                }
                AfterTurn::Remind => {
                    // 提醒注入**它自己的会话**（系统消息：用户看到的是系统行），然后**不 feed**——
                    // 泵会重问同一个人（提醒后调用计数自然重来）。
                    let evs = {
                        let mut s = self.take_single(&child)?;
                        let e = s.note_system(&reminder);
                        self.put_single(&child, s);
                        e
                    };
                    self.persister(&child).persist(&evs);
                    out.extend(evs);
                }
                AfterTurn::Unanswered => {
                    // 提醒到顶：主会话记一行"未回应"（系统消息），本轮放过它，整轮继续。
                    let mut c = self.take_collab_raw(sid)?;
                    c.pass_over(i, &mut |e| out.push(e));
                    self.sessions.insert(sid.to_string(), Session::Collab(c));
                }
            }
            // 安全网：提醒/重问必须有终点（计数有上限，这里再兜一层，防实现走偏时空转）。
            guard += 1;
            if guard > 500 {
                out.extend(self.notice_row(
                    sid,
                    "[警告] 讨论推进次数异常（已到安全上限），已停下等用户处理",
                ));
                break;
            }
        }
        Ok(out)
    }

    /// 目的：裸搬协作会话（不触发任务链派发那套副作用）：驱动循环每步都要搬一次。
    pub(crate) fn take_collab_raw(&mut self, sid: &str) -> Result<CollabSession, String> {
        match self.sessions.remove(sid) {
            Some(Session::Collab(c)) => Ok(c),
            Some(other) => {
                self.sessions.insert(sid.to_string(), other);
                Err("该会话不是协作模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：继续一次协作（同步版，CLI 与测试走这条）：**先让泵处理**（它可能把验收没过的节点退回待办），
    ///  再派发/跑完就绪节点，最后再让泵做节点验收与总验收。
    ///  生产路径是"工作线程跑泵 + put_collab 派发 + 子会话完成叫醒"，判定完全一致。
    pub fn collab_resume(&mut self, sid: &str) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        // **用户那一步**先做：重派核心指名没过的节点（唤醒不会替用户做这个决定，见 pump_with 的闸）。
        {
            let mut c = self.take_collab_raw(sid)?;
            c.resume(&mut |e| out.push(e));
            self.sessions.insert(sid.to_string(), Session::Collab(c));
        }
        // 泵（讨论回合 / 总验收）→ 派发并跑完就绪节点 → 再泵一步（总验收 → 交付）。
        out.extend(self.collab_advance(sid)?);
        out.extend(self.advance_chain(sid));
        out.extend(self.collab_advance(sid)?);
        self.record_events(sid, &mut out);
        Ok(out)
    }

    /// 目的：推进任务链：反复「派发就绪节点 → 同步跑完 → 标记完成」，直到没有可推进的。
    ///  与 Web 生产路径（ConductorHandle 起工作线程）**同一套判定**，只是这里同步做（CLI 与测试走这条）。
    pub(crate) fn advance_chain(&mut self, sid: &str) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        loop {
            let (events, todo) = self.spawn_ready_nodes(sid);
            out.extend(events);
            if todo.is_empty() {
                break;
            }
            for (node, child, objective) in todo {
                out.extend(self.drive_node(&child, &objective));
                let note = self.node_note(&child);
                self.mark_node_done(sid, &node, &note);
            }
        }
        out
    }

    /// 目的：为链里"就绪且还没有子会话"的节点建子会话并标记派发。
    ///  返回（要外送的事件, 要起生成的节点：(节点, 子会话, 任务提示词)）——
    ///  生成怎么跑由调用方决定：测试路径同步跑，生产路径交给工作线程。
    pub(crate) fn spawn_ready_nodes(
        &mut self,
        sid: &str,
    ) -> (Vec<SessionEvent>, Vec<(String, String, String)>) {
        let (ready, busy): (Vec<(String, String)>, Vec<String>) = match self.sessions.get(sid) {
            Some(Session::Collab(c)) if c.plan_approved() => {
                let chain = c.chain();
                let busy = chain
                    .map(|ch| {
                        ch.nodes
                            .iter()
                            .filter(|n| {
                                matches!(
                                    n.status,
                                    crate::capabilities::taskchain::api::NodeStatus::Running
                                )
                            })
                            .map(|n| n.assignee.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                // **只派当前阶段**：同一阶段的节点并发跑，下一阶段要等本阶段整体验收通过。
                let ready = chain
                    .map(|ch| {
                        let stage = ch.current_stage().unwrap_or(1);
                        ch.stage_ready(stage)
                            .into_iter()
                            .filter(|n| n.sub_session.is_none())
                            .map(|n| (n.id.clone(), n.assignee.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                (ready, busy)
            }
            _ => return (Vec::new(), Vec::new()),
        };
        let mut out = Vec::new();
        let mut todo = Vec::new();
        // 资源约束：**一个 agent 的会话一次只能跑一轮**——它已有在跑的节点（或本轮已派的）就跳过，
        // 下一轮推进时再派。不同 agent 不受影响，照旧并发。
        let mut taken: Vec<String> = Vec::new();
        for (node, assignee) in ready {
            if busy.contains(&assignee) || taken.contains(&assignee) {
                continue;
            }
            taken.push(assignee.clone());
            match self.spawn_sub_session(sid, &node) {
                Ok(child) => {
                    // 派发文案用册子里的执行提示词模板渲染（objective 是核心 AI 写的那段任务提示词）。
                    // 上一次的验收结论（没有 = 首轮）：**返工必须知道上次错在哪**，
                    // 否则它只能把同一件事原样再做一遍。
                    let (raw, rework) = self
                        .sessions
                        .get(sid)
                        .and_then(|s| match s {
                            Session::Collab(c) => c.chain().and_then(|ch| {
                                ch.nodes.iter().find(|n| n.id == node).map(|n| {
                                    let note = n
                                        .acceptance
                                        .as_ref()
                                        .map(|a| a.note.clone())
                                        .unwrap_or_default();
                                    let rework = if note.trim().is_empty() {
                                        String::new()
                                    } else {
                                        format!(
                                            "\n== 上次没通过的原因 ==\n{}\n这次请针对上面的原因返工。\n",
                                            note
                                        )
                                    };
                                    (n.objective.clone(), rework)
                                })
                            }),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let objective = self
                        .prompt
                        .render(Segment::ExecuteUser, &[("tasks", raw), ("rework", rework)]);
                    if let Some(Session::Collab(c)) = self.sessions.get_mut(sid) {
                        c.mark_node_started(&node, &child);
                    }
                    out.push(SessionEvent::NodeStarted {
                        node: node.clone(),
                        sid: child.clone(),
                        assignee,
                    });
                    todo.push((node, child, objective));
                }
                Err(e) => out
                    .extend(self.notice_row(sid, &format!("[错误] 节点 {} 派发失败：{}", node, e))),
            }
        }
        (out, todo)
    }

    /// 目的：为一个 agent 建它的会话（名单确认时建）：节点执行与讨论回合**共用同一个**会话。
    ///  复用"按 meta 重建"的整条装配路径——子会话与用户建的会话**没有第二种实现**。
    pub(crate) fn spawn_agent_session(
        &mut self,
        parent: &str,
        agent: &str,
    ) -> Result<String, String> {
        let (pmeta, _) = self.history.load(parent)?;
        let a = pmeta
            .agents
            .iter()
            .find(|x| x.name == agent)
            .cloned()
            .ok_or_else(|| format!("名单里没有 {}", agent))?;
        // **一个 agent 一个会话**（不是一节点一会话）：它在这场工作里的完整经历，
        // 讨论与执行不分家（见 docs/session/session-model.md）。同名即复用，幂等。
        let child = format!("{}--{}", parent, agent);
        if self.history.load(&child).is_ok() {
            return Ok(child);
        }
        let meta = SessionMeta {
            context_freshness: false,
            name: child.clone(),
            mode: "single".to_string(),
            delegate: false,
            modules: a.modules.clone(),
            // 会话跨整场工作，所以记**工作的需求**（节点目标由链记着，随回合下发）。
            task: pmeta.task.clone(),
            ts: now_ts(),
            agents: vec![a.clone()],
            exec: pmeta.exec.clone(),
            // 子会话只带自己的 narrowing；父的 ceiling 在判定时沿链求交，不复制。
            sandbox: Default::default(),
            parent: Some(parent.to_string()),
            node: None,
            delegation: None,
            thinking: None,
            run: RunState::Active,
            workspace: None,
        };
        // 沙箱锚在**顶层工作**上：该 agent 的目录在共享区那一层已经建好。
        let work = self.work_root(parent)?;
        self.workspace
            .prepare(&work, std::slice::from_ref(&a.name))?;
        self.history.create(&meta)?;
        self.ensure_session(&child)?;
        Ok(child)
    }

    /// 目的：为任务链的一个节点建**子会话**：它就是该节点负责人的 agent 会话（一个 agent 一个会话）。
    pub(crate) fn spawn_sub_session(&mut self, parent: &str, node: &str) -> Result<String, String> {
        let (pmeta, _) = self.history.load(parent)?;
        let assignee = {
            let c = match self.sessions.get(parent) {
                Some(Session::Collab(c)) => c,
                _ => return Err("无此协作会话".to_string()),
            };
            let n = c
                .chain()
                .and_then(|ch| ch.nodes.iter().find(|n| n.id == node))
                .ok_or_else(|| format!("链里没有节点 {}", node))?;
            n.assignee.clone()
        };
        // 节点的会话**就是它负责人的 agent 会话**（一个 agent 一个会话，讨论与执行不分家）。
        // 节点本身不再记在 meta 里——哪个节点正跑在这个会话里，由链的 sub_session 认。
        let _ = pmeta;
        self.spawn_agent_session(parent, &assignee)
    }

    /// 目的：代理模式：一个子会话停下了 → 往**代理会话**写一条通知行（只写这一条，不转发子会话转录）。
    ///  返回被通知的代理会话名；父不是代理会话（协作节点等）就返回 None、什么都不做。
    pub(crate) fn notify_proxy_of_child(&mut self, child: &str) -> Option<String> {
        let (meta, _) = self.history.load(child).ok()?;
        let parent = meta.parent.clone()?;
        let (pm, _) = self.history.load(&parent).ok()?;
        if pm.mode != "proxy" {
            return None;
        }
        let mut ps = self.take_single(&parent).ok()?;
        // 停在门上时把**那张卡与它的选项 id 一并说出**：代理要代答就得按选项 id 答（否则只能等用户）。
        let gate = self
            .desk_of(child)
            .queue()
            .map(|q| {
                format!(
                    "；它停在等你答的那张卡上：card={}，问「{}」，选项：{}。要代答就 send_session_message（kind=user_reply，reply=<选项 id>）",
                    q.card.id,
                    q.card.message.title,
                    q.card
                        .options
                        .iter()
                        .map(|o| o.id.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            })
            .unwrap_or_default();
        let mut evs = ps.note_task(&format!(
            "[子会话] {} 这一轮结束{}。要看它说了什么用 read_session_messages（0 = 最新）；要它继续或返工用 send_session_message。",
            child, gate
        ));
        self.put_single(&parent, ps);
        self.record_events(&parent, &mut evs);
        Some(parent)
    }
    /// 目的：生成结束**交回**：重新插入 + 解除"生成中"。
    ///  转录**已由工作线程按"一轮一次"的粒度增量落盘**（见 `Persister`），这里不重复落。
    /// 返回：若这是个**子会话**，返回它的父会话（调用方据此**叫醒父会话**推进任务链）。
    pub(crate) fn put_single(
        &mut self,
        sid: &str,
        s: crate::capabilities::session::api::AgentSession,
    ) -> Option<String> {
        self.running.remove(sid);
        self.sessions.insert(sid.to_string(), Session::Single(s));
        // 子会话完成 = 它的节点交付了：标记**正跑在这个会话里的那个节点**（产出即交付物），并交回父会话。
        // 一个 agent 一个会话（可能依次服务多个节点），所以按 sub_session 认节点，不靠 meta.node。
        let (meta, _) = self.history.load(sid).ok()?;
        let parent = meta.parent.clone()?;
        let note = self.node_note(sid);
        let node = match self.sessions.get_mut(&parent) {
            Some(Session::Collab(c)) => c.running_node_of(sid),
            _ => None,
        };
        if let Some(node) = node {
            self.mark_node_done(&parent, &node, &note);
        }
        // 代理模式的父：把"这个子会话停下了"如实告诉它（**不转发子会话转录**），
        // 由调用方按形态叫醒父会话。
        let _ = self.notify_proxy_of_child(sid);
        Some(parent)
    }

    /// 目的：同步路径的交回：整段落盘（同步跑不经过工作线程，所以没有逐轮增量落盘那一步）。
    pub(crate) fn put_single_recorded(
        &mut self,
        sid: &str,
        s: crate::capabilities::session::api::AgentSession,
        events: &[SessionEvent],
    ) {
        self.running.remove(sid);
        self.sessions.insert(sid.to_string(), Session::Single(s));
        let mut ev = events.to_vec();
        self.record_events(sid, &mut ev);
    }

    /// 目的：生成线程崩溃：会话对象随线程一起没了，但**转录在盘上**。
    ///  只解除"生成中"，下次访问按落盘转录重建——绝不把会话卡在"生成中"。
    pub(crate) fn abort_running(&mut self, sid: &str) {
        self.running.remove(sid);
    }

    /// 目的：清单即事实：每次调用重扫（策略在 conductor，机制在 ModuleSource）。
    pub fn scan(&self) -> crate::capabilities::modules::api::Roster {
        self.modules.roster()
    }
}

impl Conductor {
    /// 目的：写下本次需求（协作的起点）：需求入转录，代拟路径接着拟名单。返回期间产生的全部事件。
    pub fn collab_set_task(&mut self, sid: &str, text: &str) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        {
            let s = self.sessions.get_mut(sid).ok_or("无此会话")?;
            match s {
                Session::Collab(c) => c.set_task(text, &mut |e| out.push(e)),
                _ => return Err("该会话不是协作模式".to_string()),
            }
        }
        out.extend(self.collab_advance(sid)?);
        self.collab_tail(sid, out, None)
    }

    /// 目的：**回答一张卡**（核心线程上同步走完的那一条）：校验与出队交给队（见 `take_card`），
    ///   处置（含代拟名单的短收尾）紧跟着同步走完。
    /// 约束：生产的两条路不走它——工具级确认由等待格唤醒，长流程由调用方点火（见 `SessionOps::answer_card`）；
    ///   它给测试与"短步骤"提供一个同步入口。
    /// 返回：这一关放行了没有 + 期间产生的全部事件（已落盘）。
    #[cfg(test)]
    pub fn collab_answer(
        &mut self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<Vec<SessionEvent>, String> {
        let (answered, mut out) = self.take_card(sid, card, option, note)?;
        let Answered::Gate(ticket, _) = answered else {
            // 等在工作线程上的那一关（工具级确认）：回答已写进等待格，这里没有要推进的事。
            return Ok(out);
        };
        out.extend(self.dispose_and_advance(sid, &ticket)?);
        Ok(out)
    }

    /// 目的：把**队首那张卡的回答落定**（校验 + 出队 + 记一条回答 + 落盘）：处置由调用方接着做。
    ///   生产路径先用它把回答定死（界面立刻看到队首换人），处置再脱离调用方跑。
    /// 返回：这一答落在哪一种关上 + 要外送的事件（回答那条 + 队首换人后的新卡）。
    /// 错误：没有挂起、卡号不是队首、选项不属于那张卡——一律如实拒绝且不留痕。
    pub fn take_card(
        &mut self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<(Answered, Vec<SessionEvent>), String> {
        let Some(door) = self.desk.peek(sid) else {
            return Err("[裁决] 现在没有等你定的事。".to_string());
        };
        let answered = door.answer(card, option, note)?;
        let mut evs = match &answered {
            Answered::Waiting(e) | Answered::Gate(_, e) => e.clone(),
        };
        self.record_events(sid, &mut evs);
        Ok((answered, evs))
    }

    /// 目的：一张**已经出队**的回答处置完并推进（同步那一条：短步骤与测试走它）。
    ///   代拟名单这一关要把它定下来的名单写回 meta 并建沙箱，所以收尾留在核心线程上。
    pub fn dispose_and_advance(
        &mut self,
        sid: &str,
        ticket: &GateTicket,
    ) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        let collab = match self.sessions.get_mut(sid) {
            Some(Session::Collab(c)) => c,
            _ => return Err("该会话不是协作模式".to_string()),
        };
        let (go, confirmed) = collab.dispose(ticket, &mut |e| out.push(e))?;
        // 放行类（开工 / 重派）：走**同步**那条（泵 + 派发并跑完就绪节点 + 验收）；
        // 其余（定名单 / 请教 / 继续讨论）只需推进一步泵。生产路径两条都在工作线程上跑。
        if go {
            out.extend(self.collab_resume(sid)?);
        } else {
            out.extend(self.collab_advance(sid)?);
        }
        self.collab_tail(sid, out, confirmed)
    }

    /// 目的：当前挂着的那一队裁决（没有挂起 = None）：呈现层按它渲染，回答按**队首**认卡。
    /// 约束：判据只有这一处——队说了算（核心各关卡与工具级确认排在同一条队上），不按会话种类分叉。
    pub fn open_queue(
        &mut self,
        sid: &str,
    ) -> Result<Option<crate::capabilities::session::api::DecisionQueue>, String> {
        // 生成中的会话不在表里（对象在工作线程手里）：**队不跟着走**——工具级确认正是"生成中"问的，
        // 所以这一刻照样取得到（否则呈现层在生成中取不到卡，就答不了那一问）。
        if !self.running.contains(sid) {
            self.ensure_session(sid)?;
        }
        Ok(self.desk.peek(sid).and_then(|d| d.queue()))
    }

    /// 这一步的收尾：名单刚落档就写回 meta + 建沙箱，链就绪就派节点，终结后移出中心。
    fn collab_tail(
        &mut self,
        sid: &str,
        mut out: Vec<SessionEvent>,
        confirmed: Option<Vec<AgentMeta>>,
    ) -> Result<Vec<SessionEvent>, String> {
        if let Some(roster) = confirmed {
            let names: Vec<String> = roster.iter().map(|a| a.name.clone()).collect();
            self.workspace.prepare(sid, &names)?;
            let (mut meta, _) = self.history.load(sid)?;
            meta.agents = roster.clone();
            meta.modules = super::flat_modules(&roster);
            self.history.create(&meta)?;
            let module_roster = self.scan();
            let sandboxes = self.sandboxes(&meta, &module_roster)?;
            if let Some(Session::Collab(c)) = self.sessions.get_mut(sid) {
                c.set_sandboxes(sandboxes);
            }
        }
        // 方案过审后：就绪节点各建一个子会话——**与生产路径同一处置**，只在一边接会漏。
        // 必须在"终结后移出中心"之前做：会话一移出，链就找不到了。
        let (spawned, _) = self.spawn_ready_nodes(sid);
        out.extend(spawned);
        // 会话终结后移出中心（前端据 Ended 回收）。
        if let Some(Session::Collab(c)) = self.sessions.get(sid) {
            if c.is_done() {
                self.sessions.remove(sid);
            }
        }
        self.record_events(sid, &mut out);
        Ok(out)
    }

    /// 目的：协作中途改需求：只改需求那一行提示词——旧需求整行清掉，末尾追加一条「需求已修改」的新需求。
    ///  返回重放后的完整事件流（前端整体重建，再吸收新产生的门事件）。
    #[cfg(test)]
    pub fn update_task(&mut self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String> {
        Ok(self.update_task_outcome(sid, text)?.main)
    }

    /// 目的：改需求的完整结果——主视图 + 受影响的子会话视图（改需求不再动子会话，children 恒空）。
    /// 约束：不截断转录、不回退工作区；旧需求行删除后，新需求行追加在末尾，
    ///   这样讨论进行中的新需求也落在 [开始] 之后，成员下一轮看得到。
    pub(crate) fn update_task_outcome(
        &mut self,
        sid: &str,
        text: &str,
    ) -> Result<crate::capabilities::conductor::api::RewindOutcome, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("需求不能为空".to_string());
        }
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        self.ensure_session(sid)?;
        // @ 引用按本会话沙箱改写（与首次提交需求同一口径）。
        let text = match self.sessions.get(sid) {
            Some(Session::Collab(c)) => c.retask_text(text),
            _ => return Err("只有协作会话有「本次需求」".to_string()),
        };
        let (meta, raw) = self.history.load(sid)?;
        let next = crate::capabilities::session::api::next_line_id(&raw);
        let mut kept = Vec::with_capacity(raw.len() + 1);
        for ev in &raw {
            if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
                kept.push(ev.clone());
                continue;
            }
            let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
                kept.push(ev.clone());
                continue;
            };
            // 旧需求行按**结构化字段**认（种类=user、动词=需求），不匹配正文。
            let lines: Vec<serde_json::Value> = lines
                .iter()
                .filter(|l| {
                    !(l.get("kind").and_then(|k| k.as_str()) == Some("user")
                        && l.get("verb").and_then(|v| v.as_str()) == Some("需求"))
                })
                .cloned()
                .collect();
            if lines.is_empty() {
                continue;
            }
            let mut e2 = ev.clone();
            e2["lines"] = serde_json::Value::Array(lines);
            kept.push(e2);
        }
        let mut line = crate::capabilities::session::api::LineView::user(
            "需求",
            format!("需求已修改，以下是新需求：{}", text),
        );
        line.id = next;
        line.reply = next;
        kept.push(
            crate::capabilities::session::api::SessionEvent::Transcript(vec![line]).to_json(),
        );
        self.history
            .replace(sid, &kept)
            .map_err(|e| format!("改需求落盘失败：{}", e))?;
        let active = self.restore_session(sid, &meta, &kept, next + 1)?;
        Ok(crate::capabilities::conductor::api::RewindOutcome {
            main: active,
            children: Vec::new(),
        })
    }

    /// 目的：撤回某 agent 的「同意」：转录追加一条撤回行并提醒（模型/用户都看得到）。
    pub fn withdraw_agree(&mut self, sid: &str, agent: &str) -> Result<Vec<SessionEvent>, String> {
        self.ensure_session(sid)?;
        let mut events = Vec::new();
        match self.sessions.get_mut(sid) {
            Some(Session::Collab(c)) => c.withdraw_agree(agent, &mut |e| events.push(e)),
            Some(_) => return Err("只有协作会话有「同意」可撤回".to_string()),
            None => return Err("无此会话".to_string()),
        }
        self.record_events(sid, &mut events);
        Ok(events)
    }
}

impl Conductor {
    /// 目的：这个会话此刻挂起的是哪一关（None = 没挂起）：**队首**那一关，排队里后面的还不能答。
    /// 约束：判据是队本身（同一条队）；会话不在中心 = 如实报无此会话。
    #[cfg(test)]
    pub fn collab_pending(
        &self,
        sid: &str,
    ) -> Result<Option<crate::capabilities::session::api::Pending>, String> {
        if !self.sessions.contains_key(sid) {
            return Err("无此会话".to_string());
        }
        Ok(self
            .desk
            .peek(sid)
            .and_then(|d| d.head())
            .map(|(_, p, _)| p))
    }
}
