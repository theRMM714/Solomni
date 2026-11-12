//! 目的：协作泵：开始讨论、回答、继续，都在工作线程上跑，边产边送事件台。
//! 管：泵与主线程握手（`AskReq` / `MemberTurn`），成员回合由主线程驱动。
//! 不管：单个成员回合的实现、会话装配。
//! 联动：`api/generation/member.rs`、`docs/session/session-model.md`

use super::super::*;

impl ConductorHandle {
    /// 目的：协作的长步骤（开始讨论 / 回答 / 继续）：与单 agent 同一条 own-and-return—— 队列只占"取/交"两步，泵在工作线程上跑；事件**边产边送**事件台，界面因此能看着讨论推进。  **核心驱动**（见 docs/session/session-model.md 二之二）：泵只决定"该问谁"， 成员回合由主线程取该 agent 的会话去跑（它才拿得到那些会话）。所以泵线程与主线程**握手**： 泵让出 → 发 AskReq → 主线程跑完回 MemberTurn → 泵继续。
    pub(crate) fn collab_generation(&self, sid: &str, work: CollabWork) -> Result<Advance, String> {
        // 协作泵启动前确保这棵树的 guest 已起（代拟名单确认后第一次生成也走这里）。
        self.ensure_guest(sid)?;
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        // **先登记再取会话**：登记早于派发，所以「停止」从派发那一刻起就能生效。
        let cancel = jobs.register(sid);
        let session = match self.call({
            let sid = sid.to_string();
            move |core| core.take_collab(&sid)
        }) {
            Ok(s) => s,
            Err(e) => {
                jobs.unregister(sid, &cancel);
                return Err(e);
            }
        };
        // 把「停止」接到泵上：它在每次模型调用前与**调用中途**都看这个标志。
        let mut session = session;
        session.set_cancel(Arc::clone(&cancel));
        // 增量落盘手柄：泵产出一条定稿事件就落一条，中途刷新页面因此能看到已产生的部分。
        let persister = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.persister(&sid))
        })?;
        // 提醒上限由设置来（用户可调，见 session-model.md 二）：起线程前问一次核心。
        // 调用次数**没有上限**：模型继续核实就继续跑，直到它给出表态（或用户点停止）。
        let remind_cap = self.call(|core| Ok(core.discuss_remind_cap())).unwrap_or(3);
        let handle = self.clone();
        // 握手通道：泵 → 主线程（要一个成员回合）；主线程 → 泵（回合结果）。
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<AskReq>();
        let (turn_tx, turn_rx) = std::sync::mpsc::channel::<
            Result<crate::capabilities::collab::api::MemberTurn, String>,
        >();
        let worker = {
            let sid = sid.to_string();
            let bus = Arc::clone(&bus);
            std::thread::Builder::new()
                .name("solomni-collab".to_string())
                .spawn(move || {
                    let mut c = session;
                    let mut seq = 0u64;
                    // 安全网计数（见循环尾）：提醒/重问必须有终点，不能让泵空转。
                    let mut guard = 0usize;
                    {
                        // 边产边送 + 边落盘：长流程里用户能看着讨论一轮轮推进，
                        // 中途刷新页面也能看到已产生的部分（不再等整段结束才一次性出现）。
                        let mut sink = |ev: SessionEvent| {
                            seq = bus.push(&sid, std::slice::from_ref(&ev));
                            // 落盘失败要**如实告知**（落一条警告进事件台），不静默丢历史。
                            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                                bus.push(&sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
                            }
                        };
                        match &work {
                            // 处置一张已经出队的回答：落定（校验 / 出队 / 记账）已在核心线程上做完。
                            CollabWork::Dispose(ticket) => {
                                if let Err(e) = c.dispose(ticket, &mut sink) {
                                    sink(SessionEvent::Notice(format!("[裁决] {}", e)));
                                }
                            }
                            CollabWork::Resume => c.resume(&mut sink),
                        }
                        // 核心驱动：泵让出"该问谁"就回头找主线程（它才拿得到各 agent 的会话）。
                        loop {
                            // 先看有没有已经让出的那一步；没有就推一步（推完再看一次）。
                            let ask = match c.take_ask() {
                                Some(a) => Some(a),
                                None => {
                                    c.pump_with(&mut sink);
                                    c.take_ask()
                                }
                            };
                            let Some((i, identity, turn)) = ask else { break };
                            let Some(agent) = c.member_id(i) else { break };
                            // 提醒时要往它自己的会话里写（那边用的是同一个名字）。
                            let agent_name = agent.clone();
                            // 主会话据此显示"某某正在工作"（按钮切换与占位动画都读它）。
                            bus.push(
                                &sid,
                                &[crate::capabilities::session::api::SessionEvent::Working {
                                    agent: Some(agent_name.clone()),
                                }],
                            );
                            // **它自己的标签页同样要进"在跑"**：不然打开它只会看到上一次的运行态
                            // （或者按钮一直停在「停止」）。收尾那条在成员回合的收尾处推。
                            bus.push(
                                &format!("{}--{}", sid, agent_name),
                                &[crate::capabilities::session::api::SessionEvent::Working {
                                    agent: Some(agent_name.clone()),
                                }],
                            );
                            let req = AskReq {
                                agent,
                                identity,
                                turn,
                                systools: c.systools(),
                                cancel: c.disc_cancel(),
                                opts: c.disc_opts(),
                                round: c.round(),
                                turn_id: c.next_turn_id(),
                            };
                            let turn_id = req.turn_id;
                            if ask_tx.send(req).is_err() {
                                break;
                            }
                            // 等主线程跑完这一回合（它取会话、跑模型、落盘，再把结果送回来）。
                            let Ok(res) = turn_rx.recv() else { break };
                            match res {
                                Ok(turn) => {
                                    // **核心只提醒、不强制**（见 session-model.md 二）：
                                    // 没表态时按计数决定"注入提醒后重问"还是"记未回应后放过"。
                                    let after = c.after_member_turn(
                                        i,
                                        turn.verb.is_some(),
                                        c.cancelled(),
                                        remind_cap,
                                    );
                                    match after {
                                        crate::capabilities::collab::api::AfterTurn::Done => {
                                            c.feed_with(i, turn, turn_id, &mut sink)
                                        }
                                        crate::capabilities::collab::api::AfterTurn::Remind => {
                                            // 提醒进**它自己的会话**（系统消息）；不 feed——泵重问同一个人。
                                            let text = c.reminder_text();
                                            let child =
                                                format!("{}--{}", sid, agent_name);
                                            let target = child.clone();
                                            if let Ok(evs) = handle.call(move |core| {
                                                core.note_system(&child, &text)
                                            }) {
                                                // 提醒属于**它自己的会话**：按子会话的 sid 外送，
                                                // 不能混进主会话的事件流（否则主会话会冒出系统行）。
                                                for e in evs {
                                                    bus.push(&target, std::slice::from_ref(&e));
                                                }
                                            }
                                        }
                                        crate::capabilities::collab::api::AfterTurn::Unanswered => {
                                            c.pass_over(i, &mut sink)
                                        }
                                    }
                                    // 安全网：提醒/重问必须有终点（计数有上限，这里再兜一层）。
                                    guard += 1;
                                    if guard > 500 {
                                        sink(SessionEvent::Notice(
                                            "[警告] 讨论推进次数异常（已到安全上限），已停下等用户处理"
                                                .to_string(),
                                        ));
                                        break;
                                    }
                                }
                                Err(err) => {
                                    // 如实交回（由泵统一外送中断通知），不再往下推。
                                    c.note_turn_failure(err);
                                    c.pump_with(&mut sink);
                                    break;
                                }
                            }
                        }
                        // 泵停了（问完了 / 出错 / 被中断）：主会话回到"空闲"。
                        // 下一次问谁时会再推一条带名字的，所以这里只需要收尾这一条。
                        bus.push(
                            &sid,
                            &[crate::capabilities::session::api::SessionEvent::Working { agent: None }],
                        );
                    }
                    (c, seq)
                })
                .map_err(|e| format!("起协作线程失败：{}", e))?
        };
        // 主线程驱动每个请求：取该 agent 的会话、跑这一回合、把结果发回泵。
        // 模型调用在成员线程上（own-and-return），核心队列只占"取/交"两步——界面因此不被阻塞。
        while let Ok(req) = ask_rx.recv() {
            match self.run_member_turn(sid, &req) {
                Ok(turn) => {
                    if turn_tx.send(Ok(turn)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    // 把失败交回泵（它统一外送中断通知），不再往下推。
                    let _ = turn_tx.send(Err(err));
                    break;
                }
            }
        }
        drop(turn_tx);
        let joined = worker.join();
        jobs.unregister(sid, &cancel);
        let (c, mut seq) = match joined {
            Ok(x) => x,
            Err(_) => {
                // 线程崩了：会话对象没了，但转录在盘上——解除"生成中"，下次访问按盘重建。
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("协作线程崩溃：会话已按落盘转录保留，可继续".to_string());
            }
        };
        // 交回核心：重新插入 + **为就绪节点派发子会话**（返回派发事件与待起生成的节点）。
        let (spawned, todo) = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.put_collab(&sid, c))
        })?;
        // 协作会话停下 / 交付时也通知代理父（顶层协作没有代理父：返回 None，什么都不做）。
        if let Some(proxy) = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.notify_proxy_of_child(&sid))
        })? {
            self.spawn_detached_proxy(&proxy);
        }
        // 派发事件（"[节点] 开工"等）**也要落盘**：它们是在这里产生的，不经过上面那条 sink——
        // 只推不落的话，刷新后回放会少掉"节点开工"那几行。
        let persister = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.persister(&sid))
        })?;
        for ev in spawned {
            seq = bus.push(sid, std::slice::from_ref(&ev));
            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                bus.push(sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
            }
        }
        // 派发：每个就绪节点在**它自己的子会话**里起一轮生成（脱离本次调用，不等它跑完）。
        for (_node, child, objective) in todo {
            self.spawn_detached_node(&child, &objective);
        }
        Ok(Advance { head: seq })
    }
}
