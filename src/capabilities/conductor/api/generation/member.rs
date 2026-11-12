//! 目的：成员回合：为协作泵的一个请求取该 agent 的会话，跑完把回合结果带回来。
//! 管：子会话自己的事件台、逐轮落盘与运行态收尾。
//! 不管：协作泵的推进策略、会话装配。
//! 联动：`api/generation/collab.rs`、`api/handle.rs`

use super::super::*;

impl ConductorHandle {
    /// 目的：主线程侧：为一个成员回合取该 agent 的会话，跑完把回合结果带回来（并落进它自己的会话）。
    pub(crate) fn run_member_turn(
        &self,
        sid: &str,
        req: &AskReq,
    ) -> Result<crate::capabilities::collab::api::MemberTurn, String> {
        let child = format!("{}--{}", sid, req.agent);
        // 会话不存在就按需建（名单确认时已建，这里兜底）。
        {
            let (parent, agent, name) = (sid.to_string(), req.agent.clone(), child.clone());
            self.call(move |core| {
                if core.history_open(&name).is_err() {
                    core.spawn_agent_session(&parent, &agent)?;
                }
                Ok(())
            })?;
        }
        self.ensure_guest(&child)?;
        let session = self.call({
            let name = child.clone();
            move |core| core.take_single(&name)
        })?;
        // 思考强度是**工作根**的会话级事实：成员回合也按它解析（成员模型各自映射）。
        let thinking = self.call({
            let name = child.clone();
            move |core| core.thinking_fragment_for(&name)
        })?;
        // 这一回合的调用参数（流式 + 预算）：与单 agent 共用同一份全局设置。
        let llm = crate::capabilities::llm::api::LlmOpts {
            stream: req.opts.stream,
            timeout_secs: req.opts.timeout_secs,
            thinking: thinking.clone(),
        };
        let cancel = std::sync::Arc::clone(&req.cancel);
        let systools = req.systools.clone();
        let turn = req.turn.clone();
        let identity = req.identity.clone();
        let round = req.round;
        let turn_id = req.turn_id;
        // 子会话自己的**权威行**（逐轮）与**运行态收尾**都走它自己的事件台，
        // 并且**边产边落盘**：这个会话不经过主会话那条 sink，落盘手柄随线程带过去。
        let persister = self.call({
            let name = child.clone();
            move |core| Ok(core.persister(&name))
        })?;
        let bus = Arc::clone(&self.bus);
        let child_bus = Arc::clone(&self.bus);
        let child_sid = child.clone();
        let decisions = self.decisions_of(&child)?;
        // 工具层的提问端口：这一席的围栏装不上时经它问用户（与工具级确认同一条队）。
        let ask = decisions
            .as_ref()
            .map(|door| self.session_ask(door, &child, Arc::clone(&bus), persister.clone()));
        let joined = std::thread::Builder::new()
            .name("solomni-member".to_string())
            .spawn(move || {
                let mut s = session;
                s.set_thinking(thinking);
                // **短暂事件**（流式增量）：按子会话的 sid 外送——打开它的会话就能看到它逐字在说。
                let mut emit = {
                    let bus = Arc::clone(&bus);
                    let sid = child_sid.clone();
                    move |ev: crate::capabilities::session::api::SessionEvent| {
                        bus.push(&sid, std::slice::from_ref(&ev));
                    }
                };
                let mut live = crate::capabilities::session::api::Live {
                    llm,
                    cancel: Arc::clone(&cancel),
                    emit: &mut emit,
                    decisions,
                    ask,
                };
                // 权威行与通知也进它自己的台，并在产出的当下落盘（重建与实时同源）。
                let mut sink = |ev: crate::capabilities::session::api::SessionEvent| {
                    bus.push(&child_sid, std::slice::from_ref(&ev));
                    if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                        bus.push(
                            &child_sid,
                            std::slice::from_ref(&SessionEvent::Notice(warn)),
                        );
                    }
                };
                // 这一回合的工具面**由角色表发放**（讨论席：动词 + 只读核实工具）。
                let face = systools.role_face("discussant");
                // 这一回合：身份块（现渲染）+ 本回合工具面 + 该会话的**对话** + 开场/轮转词 + 表态。
                let ran = crate::capabilities::collab::api::discussion_turn(
                    &mut s, &identity, face, turn, turn_id, round, &mut live, &mut sink,
                );
                (s, ran)
            })
            .map_err(|e| format!("起成员线程失败：{}", e))?
            .join();
        let (s, ran) = match joined {
            Ok(x) => x,
            Err(_) => {
                self.call({
                    let name = child.clone();
                    move |core| {
                        core.abort_running(&name);
                        Ok(())
                    }
                })?;
                // 崩溃也要给子会话**收尾**：否则它的标签页永远停在"在跑"、流式层一直挂着。
                child_bus.push(&child, &[SessionEvent::Working { agent: None }]);
                child_bus.push(
                    &child,
                    &[SessionEvent::Notice(
                        crate::capabilities::session::api::interrupted_note("成员线程崩溃"),
                    )],
                );
                return Err("成员线程崩溃：该会话已按落盘转录保留".to_string());
            }
        };
        let turn = match ran {
            Ok(t) => t,
            Err(err) => {
                self.call({
                    let name = child.clone();
                    move |core| {
                        core.put_single(&name, s);
                        Ok(())
                    }
                })?;
                // 子会话也要**收尾**：失败/被停时没有定稿行，流式层必须撤下并如实说一句，
                // 否则那个标签页的光标一直挂着、按钮一直停在「停止」。
                let stopped = req.cancel.load(std::sync::atomic::Ordering::Relaxed);
                let why = if stopped {
                    crate::capabilities::session::api::stopped_note()
                } else {
                    crate::capabilities::session::api::interrupted_note(&err)
                };
                child_bus.push(&child, &[SessionEvent::Working { agent: None }]);
                child_bus.push(&child, &[SessionEvent::Notice(why)]);
                return Err(err);
            }
        };
        // 该回合的产出在跑的时候就已经落进**它自己的会话**并入了它自己的事件台（逐轮、逐条落盘）；
        // 这里只做运行态收尾：主会话有 working{None}，子会话也要有——否则那个标签页一直显示"正在工作"、按钮一直停在「停止」。
        child_bus.push(
            &child,
            &[crate::capabilities::session::api::SessionEvent::Working { agent: None }],
        );
        self.call({
            let name = child.clone();
            move |core| {
                core.put_single(&name, s);
                Ok(())
            }
        })?;
        Ok(turn)
    }
}
