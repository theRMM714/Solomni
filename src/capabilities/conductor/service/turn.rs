//! 目的：单 agent 的回合收发——工具形态刷新、生成前准备、节点派发准备、同步生成入口、继续。
//! 管：一次单 agent 回合的判据（形态变了先提示、继续时末条必须是用户发言）。
//! 不管：拟名单（suggest.rs）、协作动作（flow.rs）、会话启用（activation.rs）。
//! 联动：句柄层的工作线程与 flow 的节点派发经这里取准备结果。
//!   本方法族只碰 sessions / history / registry / running 这几个字段。

use super::*;

impl Conductor {
    /// 目的：形态**不钉在会话里**：每次生成前按登记处重新解析。
    ///  变了 → 只改会话参数里的那一格并给用户一句通知（身份块下次调用就按新约定渲染）；没变 → 什么都不做。
    ///  这样"用户改了登记处就重新查、没改就不管"，同时身份块里的约定与实际协议始终一致。
    pub(crate) fn refresh_tool_mode(&mut self, sid: &str) -> Result<Option<String>, String> {
        let a = match self.sessions.get(sid) {
            Some(Session::Single(_)) => {
                let (meta, _) = self.history.load(sid)?;
                match meta.agents.first().cloned() {
                    Some(a) => a,
                    None => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        // 形态从登记处派生；通道没了就是前置条件失败，不悄悄退回手写信封。
        self.registry
            .channel(a.model.as_deref())
            .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
        let want = self.registry.tool_mode(a.model.as_deref());
        let cur = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.tool_mode(),
            // 还没装进内存的会话：交给 ensure_session 按当前形态建，这里不动
            _ => want,
        };
        if cur == want {
            return Ok(None);
        }
        // **只改这一格**：形态是登记处派生出来的参数，没必要把整个会话从盘上重建一遍
        //（身份块每次调用现渲染，改完这一格下一回合就生效）。
        if let Some(Session::Single(s)) = self.sessions.get_mut(sid) {
            s.set_tool_mode(want);
        }
        Ok(Some(match want {
            crate::capabilities::llm::api::ToolMode::Native => {
                "工具调用形态已按登记处改为**原生工具调用**（本条起生效）".to_string()
            }
            crate::capabilities::llm::api::ToolMode::Envelope => {
                "工具调用形态已按登记处改为**手写信封**（本条起生效）".to_string()
            }
        }))
    }

    /// 目的：生成前的**准备**（短命令：只做检查与取出会话，不跑模型）。
    ///  语义：工具形态变了先给一句提示；
    ///  继续时末条必须是用户发言（否则只提醒，不替用户发言）。
    pub(crate) fn prepare_single(
        &mut self,
        sid: &str,
        text: Option<&str>,
        want_stream: bool,
    ) -> Result<Prepared, String> {
        // 还没装进内存的会话先从落盘重建（"继续"可能先于"打开"到达；真没这个会话仍然报无此会话）。
        self.ensure_session(sid)?;
        if matches!(self.sessions.get(sid), Some(Session::Collab(_))) {
            return Ok(Prepared::NotSingle);
        }
        let mut prefix: Vec<SessionEvent> = Vec::new();
        if let Some(n) = self.refresh_tool_mode(sid)? {
            prefix.push(SessionEvent::Notice(n));
        }
        if text.is_none() {
            let last_is_user =
                matches!(self.sessions.get(sid), Some(Session::Single(s)) if s.last_is_user());
            if !last_is_user {
                prefix.push(SessionEvent::Notice(NEED_USER.to_string()));
                return Ok(Prepared::Immediate(prefix));
            }
        }
        // 思考强度按**工作根**解析（子会话共享）；模型没登记该档会在这里如实报错。
        let llm = self.llm_opts_for(sid, want_stream)?;
        // 身份块**现渲染**（形态刚在上一步对齐过，所以这里读到的就是本回合的形态）。
        let identity = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.params().identity(&*self.prompt, s.tool_mode()),
            _ => return Ok(Prepared::NotSingle),
        };
        let mut session = self.take_single(sid)?;
        self.refresh_context_budget(&mut session);
        session.set_thinking(llm.thinking.clone());
        let persister = self.persister(sid);
        Ok(Prepared::Run {
            session: Box::new(session),
            identity,
            prefix,
            task: None,
            llm,
            persister,
        })
    }

    /// 目的：起一个节点的执行回合（Web 生产路径）：**核心注入派发任务**（界面系统行、上下文 user 角色），
    ///  再把会话交给工作线程继续生成。CLI 的 `drive_node` 与本条是同一条语义——
    ///  各前端只做各自的界面，节点派发只有这一条管道。
    ///  界面的身份与进上下文的角色是两件事：界面上它是核心说的话（不得显示成"用户"），
    ///  而上下文里必须有一条 user 回合，否则请求被供应商整条拒收（见 session-model.md 四之二）。
    pub(crate) fn prepare_node(
        &mut self,
        child: &str,
        objective: &str,
    ) -> Result<Prepared, String> {
        self.ensure_session(child)?;
        if matches!(self.sessions.get(child), Some(Session::Collab(_))) {
            return Ok(Prepared::NotSingle);
        }
        // 节点的生成跟随设置里的流式开关；思考强度按工作根解析（子会话共享）。
        let llm = self.llm_opts_for(child, true)?;
        let mut prefix: Vec<SessionEvent> = Vec::new();
        if let Some(n) = self.refresh_tool_mode(child)? {
            prefix.push(SessionEvent::Notice(n));
        }
        let mut session = self.take_single(child)?;
        self.refresh_context_budget(&mut session);
        session.set_thinking(llm.thinking.clone());
        let identity = session
            .params()
            .identity(&*self.prompt, session.tool_mode());
        let persister = self.persister(child);
        // 派发行由工作线程在**压缩之后**注入（`dispatch_task`）：这样本轮任务不会被压进摘要。
        Ok(Prepared::Run {
            session: Box::new(session),
            identity,
            prefix,
            task: Some(objective.to_string()),
            llm,
            persister,
        })
    }

    /// 目的：测试用同步入口：与工作线程那条路**同一段语义**（准备 → 生成 → 交回落盘）。
    ///  生产路径的生成在工作线程上（见 `ConductorHandle::single_generation`）。
    #[cfg(test)]
    pub fn single_say(
        &mut self,
        sid: &str,
        text: &str,
        live: &mut Live,
    ) -> Result<Vec<SessionEvent>, String> {
        match self.prepare_single(sid, Some(text), live.llm.stream)? {
            Prepared::Immediate(events) => Ok(events),
            Prepared::NotSingle => Err("该会话不是单 agent 模式".to_string()),
            Prepared::Run {
                session,
                identity,
                prefix,
                ..
            } => {
                let mut session = *session;
                let mut events = prefix;
                {
                    let mut sink = |ev: SessionEvent| events.push(ev);
                    crate::capabilities::collab::api::say(
                        &mut session,
                        text,
                        &identity,
                        live,
                        &mut sink,
                    );
                }
                if live.cancelled() {
                    self.log.warn("conductor::single_say", "生成被用户中止");
                }
                self.put_single_recorded(sid, session, &events);
                Ok(events)
            }
        }
    }
}

impl Conductor {
    /// 目的：继续：由用户点击授权。单 agent 会话需要轮到用户（末条是 AI 就只提醒、不发请求）；
    ///  协作不需要用户发言，继续 = 从断点推进流水线。
    ///  **测试用同步入口**：生产路径的两条（单 agent / 协作）都在工作线程上跑（见 ConductorHandle）。
    #[cfg(test)]
    pub fn continue_flow(
        &mut self,
        sid: &str,
        live: &mut Live,
    ) -> Result<Vec<SessionEvent>, String> {
        self.ensure_session(sid)?;
        let mut events = {
            let s = self.sessions.get_mut(sid).ok_or("无此活动会话")?;
            match s {
                Session::Single(s) => {
                    if s.last_is_user() {
                        let mut out = Vec::new();
                        {
                            let mut sink = |ev: SessionEvent| out.push(ev);
                            let identity = s.params().identity(&*self.prompt, s.tool_mode());
                            crate::capabilities::collab::api::continue_reply(
                                s, &identity, live, &mut sink,
                            );
                        }
                        out
                    } else {
                        vec![SessionEvent::Notice(NEED_USER.to_string())]
                    }
                }
                // 协作不需要用户发言：从断点推进流水线。
                Session::Collab(c) => {
                    let mut out = Vec::new();
                    c.resume(&mut |e| out.push(e));
                    out
                }
            }
        };
        self.record_events(sid, &mut events);
        Ok(events)
    }
}
