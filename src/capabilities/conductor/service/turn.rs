//! **回合收发与协作动作**：拟名单分发、单 agent 的话与准备、协作推进入口、会话生命周期（ensure_session）。
//!
//! 名单由 slate 业务拟，这里只做队列分发与呈现映射。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::*;

impl Conductor {
    /// 核心推荐：按本次需求推荐 agent 名单（**拟名单业务**的用例；这里只做队列分发、把行收出来）。
    /// 前置判据（登记处有没有模型 / 核心默认设没设）放在这里：它是对用户的提示，不是名单的合法性。
    pub fn suggest_models(
        &self,
        task: &str,
        mode: WorkMode,
    ) -> Result<(Vec<AgentSuggestion>, Vec<SessionEvent>), String> {
        if !self.registry.any_models() {
            return Err("登记处还没有任何模型，请先到「模型登记」添加".to_string());
        }
        let channel = self.registry.core_channel().ok_or_else(|| {
            "核心未设定默认模型（或它引用的供应商不存在），请先到「核心 AI 默认模型」设定"
                .to_string()
        })?;
        let roster = self.scan();
        let (mut chat, _) = self.llm.core_channel(Some(&channel));
        // 登记处事实：**自持一份快照**（拟名单只读；一次用户动作、代价可忽略，
        // 与协作会话自持一份同义——写面是 `Box<dyn Registry>`，不可能共享出去）。
        let facts = self.registry.snapshot();
        // 核心这一趟的行（工具行、发言行、思维链）**一律外送**：核心没有会话、写不了盘，
        // 它只把行交出来；推到哪个 sid、落不落盘由调用方按会话种类定（这里给系统会话）。
        let mut rows: Vec<SessionEvent> = Vec::new();
        let proposal = crate::capabilities::slate::api::propose(
            &crate::capabilities::slate::api::Parties {
                prompt: &*self.prompt,
                tools: &*self.systools,
                roster: &roster,
                agents: &facts.agents,
                models: &facts.models,
            },
            &mut crate::capabilities::slate::api::Request {
                task,
                mode: match mode {
                    WorkMode::Single => crate::capabilities::slate::api::Mode::Single,
                    WorkMode::Collab => crate::capabilities::slate::api::Mode::Collab,
                    // 代理形态没有名单可拟（核心自己挑人）；如实拒绝，不拿单模式的清单糊弄。
                    WorkMode::Proxy => {
                        return Err("代理形态没有名单：决定权整块交给核心，由它自己挑人".to_string())
                    }
                },
                chat: chat.as_mut(),
                tool_mode: self.registry.tool_mode(None),
                opts: crate::capabilities::llm::api::CompleteOpts::plain(false),
                cancel: None,
                // 推荐是**一次性建议**（用户点了才生成、没有工作区可核实）：不接核实回路。
                verify: None,
                sink: &mut |e: SessionEvent| rows.push(e),
            },
        )?;
        for r in &proposal.rejected {
            self.log.warn(
                "conductor::suggest_models",
                &format!("拟名单条目拒收：{}", r),
            );
        }
        // 核心只建议、不代选：名单里的每一项都逐条核验过（存在性 / 模型真实 / 模块不重复）。
        let out: Vec<AgentSuggestion> = proposal
            .picks
            .into_iter()
            .map(|p| AgentSuggestion {
                reuse: !p.agent.transient,
                name: p.agent.name,
                // 复用项没有自己的模型时，落到核心默认（前端仍可改）。
                model: p
                    .agent
                    .model
                    .or_else(|| facts.core.clone())
                    .unwrap_or_default(),
                modules: p.agent.modules,
                why: p.why,
            })
            .collect();
        if out.is_empty() {
            return Err("核心推荐没有可用结果".to_string());
        }
        Ok((out, rows))
    }

    // ---- 会话收发（前端永不接触会话本体） ----

    /// 形态**不钉在会话里**：每次生成前按登记处重新解析。
    /// 变了 → 只改会话参数里的那一格并给用户一句通知（身份块下次调用就按新约定渲染）；没变 → 什么都不做。
    /// 这样"用户改了登记处就重新查、没改就不管"，同时身份块里的约定与实际协议始终一致。
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
        let channel = self.registry.channel(a.model.as_deref());
        let want = if channel.is_some() {
            self.registry.tool_mode(a.model.as_deref())
        } else {
            crate::capabilities::llm::api::ToolMode::Envelope
        };
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

    /// 生成前的**准备**（短命令：只做检查与取出会话，不跑模型）。
    /// 语义：工具形态变了先给一句提示；
    /// 继续时末条必须是用户发言（否则只提醒，不替用户发言）。
    pub(crate) fn prepare_single(
        &mut self,
        sid: &str,
        text: Option<&str>,
        want_stream: bool,
    ) -> Result<Prepared, String> {
        let llm = self.llm_opts(want_stream);
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
        // 身份块**现渲染**（形态刚在上一步对齐过，所以这里读到的就是本回合的形态）。
        let identity = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.params().identity(&*self.prompt, s.tool_mode()),
            _ => return Ok(Prepared::NotSingle),
        };
        let session = self.take_single(sid)?;
        let persister = self.persister(sid);
        Ok(Prepared::Run {
            session: Box::new(session),
            identity,
            prefix,
            llm,
            persister,
        })
    }

    /// 起一个节点的执行回合（Web 生产路径）：**核心注入派发任务**（界面系统行、上下文 user 角色），
    /// 再把会话交给工作线程继续生成。CLI 的 `drive_node` 与本条是同一条语义——
    /// 各前端只做各自的界面，节点派发只有这一条管道。
    /// 界面的身份与进上下文的角色是两件事：界面上它是核心说的话（不得显示成"用户"），
    /// 而上下文里必须有一条 user 回合，否则请求被供应商整条拒收（见 session-model.md 四之二）。
    pub(crate) fn prepare_node(
        &mut self,
        child: &str,
        objective: &str,
    ) -> Result<Prepared, String> {
        // 节点的生成跟随设置里的流式开关。
        let llm = self.llm_opts(true);
        self.ensure_session(child)?;
        if matches!(self.sessions.get(child), Some(Session::Collab(_))) {
            return Ok(Prepared::NotSingle);
        }
        let mut prefix: Vec<SessionEvent> = Vec::new();
        if let Some(n) = self.refresh_tool_mode(child)? {
            prefix.push(SessionEvent::Notice(n));
        }
        let mut session = self.take_single(child)?;
        // 核心注入：派发行（界面系统行、上下文 user 角色），这里只记录、不跑模型——
        // 生成交给工作线程（与 CLI 的 `drive_node` 同一段语义，见 session-model.md 四之二）。
        prefix.extend(session.note_task(objective));
        let identity = session
            .params()
            .identity(&*self.prompt, session.tool_mode());
        let persister = self.persister(child);
        Ok(Prepared::Run {
            session: Box::new(session),
            identity,
            prefix,
            llm,
            persister,
        })
    }

    /// 测试用同步入口：与工作线程那条路**同一段语义**（准备 → 生成 → 交回落盘）。
    /// 生产路径不再走它——那里的生成在工作线程上（见 `ConductorHandle::single_generation`）。
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

    /// 目的：**回答一张裁决卡**（核心线程上的那一半）：校验选项属于当时那张卡，再按选项 id 分派。
    ///   代拟名单这一关要把它定下来的名单写回 meta 并建沙箱，所以它留在核心线程上收尾。
    pub fn collab_answer(
        &mut self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        let mut confirmed: Option<Vec<AgentMeta>> = None;
        let go = {
            let s = self.sessions.get_mut(sid).ok_or("无此会话")?;
            let collab = match s {
                Session::Collab(c) => c,
                _ => return Err("该会话不是协作模式".to_string()),
            };
            let empty_before = collab.roster().is_empty();
            let go = collab.answer_card(card, option, note, &mut |e| out.push(e))?;
            // 名单刚由这一答定下来：接下来要把 roster 写回 meta（重建与沙箱归属都读它）。
            if empty_before && !collab.roster().is_empty() {
                confirmed = Some(collab.roster().to_vec());
            }
            go
        };
        // 放行类（开工 / 重派）：走**同步**那条（泵 + 派发并跑完就绪节点 + 验收）；
        // 其余（定名单 / 请教 / 继续讨论）只需推进一步泵。生产路径两条都在工作线程上跑。
        if go {
            out.extend(self.collab_resume(sid)?);
        } else {
            out.extend(self.collab_advance(sid)?);
        }
        self.collab_tail(sid, out, confirmed)
    }

    /// 目的：测试用：代拟拟好的名单（生产路径不取它——名单在转录的 [代拟] 行里，界面照那份显示）。
    #[cfg(test)]
    pub fn collab_slate(&mut self, sid: &str) -> Result<Vec<AgentMeta>, String> {
        self.ensure_session(sid)?;
        match self.sessions.get(sid) {
            Some(Session::Collab(c)) => Ok(c.slate()),
            Some(_) => Err("该会话不是协作模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：当前挂起是哪一关（None = 没在等门）：回答走"短步骤"还是"点火跑泵"按它分。
    pub fn collab_gate_kind(&mut self, sid: &str) -> Result<Option<String>, String> {
        // 会话不在中心 / 不是协作会话：如实给 None——真正的拒绝由回答那一步报出来。
        Ok(match self.collab_pending(sid) {
            Ok(p) => p.map(|p| p.kind().to_string()),
            Err(_) => None,
        })
    }

    /// 目的：当前挂着的那张卡（没有挂起 = None）：呈现层按它渲染，回答按它认卡。
    pub fn collab_open_card(
        &mut self,
        sid: &str,
    ) -> Result<Option<crate::capabilities::session::api::DecisionCard>, String> {
        self.ensure_session(sid)?;
        match self.sessions.get(sid) {
            Some(Session::Collab(c)) => Ok(c.open_card()),
            Some(_) => Err("该会话不是协作模式".to_string()),
            None => Err("无此会话".to_string()),
        }
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
            meta.modules = roster.iter().flat_map(|a| a.modules.clone()).collect();
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

    /// 协作中途改需求：回到需求行并追加一条新需求（旧需求留在流水里，派生以最后一条为准）。
    /// 返回重放后的完整事件流（前端整体重建，再吸收新产生的门事件）。
    pub fn update_task(&mut self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("需求不能为空".to_string());
        }
        self.ensure_session(sid)?;
        let (_, events) = self.history_open(sid)?;
        // 需求行按**结构化字段**认（种类=user、动词=需求），不匹配正文。
        let keep = crate::capabilities::session::api::find_line_id(&events, |l| {
            l.kind == "user" && l.verb == "需求"
        })
        .ok_or("该会话没有需求行")?;
        // 回档语义是「保留 id < keep」：需求行本身要留下（新需求随后追加），所以传 keep + 1。
        // 改需求走**删除**模式：真的截掉旧需求之后的派生，再按新需求重新展开。
        let mut out = self.rewind(sid, RewindTarget::Delete(keep + 1))?;
        let mut fresh = Vec::new();
        {
            let s = self.sessions.get_mut(sid).ok_or("无此会话")?;
            match s {
                Session::Collab(c) => c.set_task(text, &mut |e| fresh.push(e)),
                _ => return Err("只有协作会话有「本次需求」".to_string()),
            }
        }
        self.record_events(sid, &mut fresh);
        out.extend(fresh.iter().map(|e| e.to_json()));
        Ok(out)
    }

    /// 撤回某 agent 的「同意」：转录追加一条撤回行并提醒（模型/用户都看得到）。
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

    /// 历史会话转为活动会话（跨重启续跑/回档的前提）：按转录重建，状态全部派生。
    pub(crate) fn ensure_session(&mut self, sid: &str) -> Result<(), String> {
        if self.sessions.contains_key(sid) {
            return Ok(());
        }
        // 会话对象在工作线程上（生成中）：**不能**从盘上再建一份（会变成两个实例）。
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, events) = self.history_open(sid)?;
        let rebuilt = self.rebuild_session(&meta, &events)?;
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(())
    }

    /// 继续：由用户点击授权。单 agent 会话需要轮到用户（末条是 AI 就只提醒、不发请求）；
    /// 协作不需要用户发言，继续 = 从断点推进流水线。
    /// **测试用同步入口**：生产路径的两条（单 agent / 协作）都在工作线程上跑（见 ConductorHandle）。
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

    /// 协作会话当前介入请求（None = 无挂起或已终结）。
    pub fn collab_pending(&self, sid: &str) -> Result<Option<Pending>, String> {
        match self.sessions.get(sid) {
            Some(Session::Collab(c)) => Ok(c.pending.clone()),
            Some(_) => Err("该会话不是协作模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }
}
