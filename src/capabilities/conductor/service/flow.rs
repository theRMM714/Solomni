//! **协作流水线推进**：链推进（advance_chain）、节点派发（drive_node / spawn_ready_nodes / spawn_sub_session）、协作步（collab_advance / collab_resume）。
//!
//! 它是协作业务在协调侧的那一段编排：取对象、推进、放回（take_* / put_*）。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::*;

impl Conductor {
    /// **同步**跑一个节点的子会话（CLI 与测试走这条；Web 生产路径由 ConductorHandle 起工作线程）。
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
                        // 同步驱动、生成期间读不到键盘：这一趟不接工具级确认。
                        decisions: None,
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

    /// 推进协作（**核心驱动**）：泵只决定"该问谁"，核心取该 agent 的会话跑这一回合再交回。
    /// 契约见 docs/session/session-model.md 二之二。事件由调用方统一落档。
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
                    },
                    cancel: std::sync::Arc::clone(&cancel),
                    emit: &mut |e: SessionEvent| notes.borrow_mut().push(e),
                    decisions: None,
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
                    out.push(SessionEvent::Notice(note));
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
                out.push(SessionEvent::Notice(
                    "[警告] 讨论推进次数异常（已到安全上限），已停下等用户处理".to_string(),
                ));
                break;
            }
        }
        Ok(out)
    }

    /// 裸搬协作会话（不触发任务链派发那套副作用）：驱动循环每步都要搬一次。
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

    /// 继续一次协作（同步版，CLI 与测试走这条）：**先让泵处理**（它可能把验收没过的节点退回待办），
    /// 再派发/跑完就绪节点，最后再让泵做节点验收与总验收。
    /// 生产路径是"工作线程跑泵 + put_collab 派发 + 子会话完成叫醒"，判定完全一致。
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

    /// 推进任务链：反复「派发就绪节点 → 同步跑完 → 标记完成」，直到没有可推进的。
    /// 与 Web 生产路径（ConductorHandle 起工作线程）**同一套判定**，只是这里同步做（CLI 与测试走这条）。
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

    /// 为链里"就绪且还没有子会话"的节点建子会话并标记派发。
    /// 返回（要外送的事件, 要起生成的节点：(节点, 子会话, 任务提示词)）——
    /// 生成怎么跑由调用方决定：测试路径同步跑，生产路径交给工作线程。
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
                Err(e) => out.push(SessionEvent::Notice(format!(
                    "[错误] 节点 {} 派发失败：{}",
                    node, e
                ))),
            }
        }
        (out, todo)
    }

    /// 为一个 agent 建它的会话（名单确认时建）：节点执行与讨论回合**共用同一个**会话。
    /// 复用"按 meta 重建"的整条装配路径——子会话与用户建的会话**没有第二种实现**。
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
            name: child.clone(),
            mode: "single".to_string(),
            delegate: false,
            modules: a.modules.clone(),
            // 会话跨整场工作，所以记**工作的需求**（节点目标由链记着，随回合下发）。
            task: pmeta.task.clone(),
            ts: now_ts(),
            agents: vec![a.clone()],
            exec: pmeta.exec.clone(),
            parent: Some(parent.to_string()),
            node: None,
            delegation: None,
            run: RunState::Active,
        };
        // 沙箱锚在**顶层工作**上：该 agent 的目录在共享区那一层已经建好。
        let work = self.work_root(parent)?;
        self.workspace
            .prepare(&work, std::slice::from_ref(&a.name))?;
        self.history.create(&meta)?;
        self.ensure_session(&child)?;
        Ok(child)
    }

    /// 为任务链的一个节点建**子会话**：它就是该节点负责人的 agent 会话（一个 agent 一个会话）。
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

    /// 代理模式：一个子会话停下了 → 往**代理会话**写一条通知行（只写这一条，不转发子会话转录）。
    /// 返回被通知的代理会话名；父不是代理会话（协作节点等）就返回 None、什么都不做。
    pub(crate) fn notify_proxy_of_child(&mut self, child: &str) -> Option<String> {
        let (meta, _) = self.history.load(child).ok()?;
        let parent = meta.parent.clone()?;
        let (pm, _) = self.history.load(&parent).ok()?;
        if pm.mode != "proxy" {
            return None;
        }
        let mut ps = self.take_single(&parent).ok()?;
        let mut evs = ps.note_task(&format!(
            "[子会话] {} 这一轮结束。要看它说了什么用 read_session_messages（0 = 最新）；要它继续或返工用 send_session_message。",
            child
        ));
        self.put_single(&parent, ps);
        self.record_events(&parent, &mut evs);
        Some(parent)
    }
    /// 生成结束**交回**：重新插入 + 解除"生成中"。
    /// 转录**已由工作线程按"一轮一次"的粒度增量落盘**（见 `Persister`），这里不重复落。
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

    /// 同步路径的交回：整段落盘（同步跑不经过工作线程，所以没有逐轮增量落盘那一步）。
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

    /// 生成线程崩溃：会话对象随线程一起没了，但**转录在盘上**。
    /// 只解除"生成中"，下次访问按落盘转录重建——绝不把会话卡在"生成中"。
    pub(crate) fn abort_running(&mut self, sid: &str) {
        self.running.remove(sid);
    }

    /// 清单即事实：每次调用重扫（策略在 conductor，机制在 ModuleSource）。
    pub fn scan(&self) -> crate::capabilities::workspace::api::Roster {
        self.workspace.roster()
    }
}
