//! 目的：代理入站桥：把 proxy::api 的 ProxyHost 六件事翻译成核心手柄的命令。
//! 管：ProxyBridge（唯一出站端口的宿主实现）与装配点 inject_proxy_handler，以及核心侧的代理视图（目录 / 观察 / 消息 / 控制）。
//! 不管：代理协议本身（在 proxy 能力）；本文件只做参数搬运与调用者身份（Caller::Role）的构造。
//! 联动：proxy 的成员侧执行者经本桥回调；核心线程由 Handle::call 进入。

use super::*;

fn tool_mode_str(m: crate::capabilities::llm::api::ToolMode) -> String {
    match m {
        crate::capabilities::llm::api::ToolMode::Native => "native".to_string(),
        crate::capabilities::llm::api::ToolMode::Envelope => "envelope".to_string(),
    }
}

impl Core {
    /// 一个会话的转录行（回放口径）：观察计数与消息倒查共用，避免两处各解析一遍。
    fn proxy_lines(
        &self,
        sid: &str,
    ) -> Result<Vec<crate::capabilities::conversation::session::api::LineView>, String> {
        let (_, events) = self.lifecycle.history_open(sid)?;
        let mut out = Vec::new();
        for ev in &events {
            if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
                continue;
            }
            let Some(arr) = ev.get("lines").and_then(|l| l.as_array()) else {
                continue;
            };
            for l in arr {
                if let Ok(v) = serde_json::from_value::<
                    crate::capabilities::conversation::session::api::LineView,
                >(l.clone())
                {
                    out.push(v);
                }
            }
        }
        Ok(out)
    }

    /// 产物索引：共享区与各 agent 沙箱里的**相对路径**（不泄漏真实私有路径）。
    fn proxy_artifacts(&self, sid: &str) -> Result<Vec<String>, String> {
        let files = self.files_view(sid)?;
        let mut out: Vec<String> = files.work.iter().map(|p| format!("work/{}", p)).collect();
        for a in &files.agents {
            for p in &a.files {
                out.push(format!("{}/{}", a.name, p));
            }
        }
        Ok(out)
    }

    /// 目的：代理工具：只读清单（agent / 模块 / 模型），只回公开事实，不碰密钥与私有路径。
    pub fn proxy_catalog(&self, scope: d::CatalogScope) -> Result<d::Catalog, String> {
        let agents = if scope.covers_agents() {
            self.registry().agent_views()
        } else {
            Vec::new()
        };
        let modules = if scope.covers_modules() {
            self.modules
                .roster()
                .modules
                .iter()
                .map(|m| d::ModuleFact {
                    id: m.manifest.id.clone(),
                    tools: m.manifest.tools.keys().cloned().collect(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let models = if scope.covers_models() {
            self.registry()
                .model_views()
                .into_iter()
                .map(|v| d::ModelFact {
                    id: v.id,
                    name: v.name,
                    tools: tool_mode_str(v.tools),
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(d::Catalog {
            agents,
            modules,
            models,
        })
    }

    /// 目的：代理工具：观察（元信息；**不回消息正文**）。
    pub fn proxy_observe(
        &self,
        sid: &str,
        view: d::ObserveView,
        since: Option<&str>,
    ) -> Result<d::Snapshot, String> {
        // 存在性与运行态都取 meta（**不回放流水**）；"此刻在不在跑"再看会话中心。
        let meta = self
            .history
            .meta(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let list = self.lifecycle.history_list();
        let sv = self.session_views(&list).into_iter().find(|v| v.sid == sid);
        let state = match meta.run {
            RunState::Active => match &sv {
                Some(v) if v.running => "running",
                Some(v) if v.done => "done",
                _ => "idle",
            }
            .to_string(),
            // 运行态（持久事实）优先：它比"此刻在不在跑"更能说明这条会话为什么不往前走。
            other => other.as_str().to_string(),
        };
        let want = |w: d::ObserveView| view == d::ObserveView::Full || view == w;
        let pending = if want(d::ObserveView::Pending) {
            sv.as_ref()
                .and_then(|v| v.pending.clone())
                .map(|p| vec![p.to_string()])
        } else {
            None
        };
        let artifacts = if want(d::ObserveView::Artifacts) {
            Some(self.proxy_artifacts(sid)?)
        } else {
            None
        };
        let message_count = Some(self.proxy_lines(sid)?.len());
        // 游标 = 消息条数。`since` 只用来给"自上次以来新增几条"这个增量信号；
        // 回执其余部分是当前事实的**幂等快照**（调用方不因此漏读，也不因此重复读）。
        let new_messages = match (
            since.and_then(|s| s.trim().parse::<usize>().ok()),
            message_count,
        ) {
            (Some(prev), Some(now)) => Some(now.saturating_sub(prev)),
            _ => None,
        };
        Ok(d::Snapshot {
            session: sid.to_string(),
            state: state.to_string(),
            pending,
            artifacts,
            message_count,
            cursor: message_count.map(|n| n.to_string()),
            new_messages,
        })
    }

    /// 目的：代理工具：消息倒查（0 = 最新一条，按新→旧）。
    pub fn proxy_messages(
        &self,
        sid: &str,
        from: usize,
        count: usize,
    ) -> Result<d::MessagesPage, String> {
        let lines = self.proxy_lines(sid)?;
        let total = lines.len();
        let mut out = Vec::new();
        let mut i = from;
        while i < total && out.len() < count {
            let l = &lines[total - 1 - i];
            out.push(d::MessageLine {
                id: l.id,
                speaker: l.speaker.clone(),
                verb: l.verb.clone(),
                kind: l.kind.clone(),
                text: l.line.clone(),
            });
            i += 1;
        }
        let next = if i < total { Some(i) } else { None };
        Ok(d::MessagesPage {
            session: sid.to_string(),
            messages: out,
            next,
        })
    }

    /// 目的：一个会话的形态（single / collab）：落盘 meta 是唯一真相；取不到就回空串。
    pub(crate) fn session_mode_str(&self, sid: &str) -> String {
        self.lifecycle
            .history_open(sid)
            .map(|(m, _)| m.mode)
            .unwrap_or_default()
    }

    /// 目的：代理工具：控制里的**终态关闭**（只做运行态的状态转移，不启动也不停止生成——
    ///  停止 / 继续要 JobRegistry 与唤醒，只有句柄拿得到，见 `ProxyBridge::control`）。
    ///  可回放记录也由句柄那一侧写（它才推得到事件台）。
    ///  - close：终态。整棵子树里还有在生成的就拒绝（先 stop），不留半个状态；
    ///  - stop / continue：级联停与解冻唤醒由句柄承担，不进这条。
    pub fn proxy_control(
        &self,
        sid: &str,
        action: d::ControlAction,
    ) -> Result<d::ControlState, String> {
        let meta = self.history.meta(sid)?;
        let target = meta.name.clone();
        match action {
            d::ControlAction::Stop | d::ControlAction::Continue => {
                return Err(format!(
                    "{} 不走这条：运行态与生成由句柄承担",
                    action.as_str()
                ));
            }
            d::ControlAction::Close => {
                // 「关闭已完成或已停止的会话」：子树里还有在生成的就整条拒绝。
                for s in subtree_of(self.history.as_ref(), sid) {
                    if self.turn.is_running(&s) {
                        return Err(Turn::running_refusal(&s));
                    }
                }
                if meta.run != RunState::Closed {
                    let mut m = meta.clone();
                    m.run = RunState::Closed;
                    self.history.save_meta(&m)?;
                }
            }
        };
        Ok(d::ControlState {
            session: target,
            action,
            state: "closed".to_string(),
        })
    }

    /// 目的：测试专用：造一条代理父会话（不经动作表）。
    #[cfg(test)]
    pub fn create_proxy(&mut self, name: &str, granted_at: i64) -> Result<String, String> {
        self.work
            .create_proxy_with_facts(
                &mut self.turn,
                self.registry.as_ref(),
                self.probe.as_ref(),
                crate::capabilities::work::api::ProxySeed {
                    name,
                    granted_at,
                    tier: crate::kernel::api::Tier::Host,
                    guest: None,
                    profile: crate::capabilities::permission::api::ProfileId::AllReach,
                    workspace: None,
                },
            )
            .map(|(sid, _)| sid)
    }
}

/// 目的：代理工具的**队列宿主**：把 ProxyHost 的每个方法交给核心线程执行。
///  为什么需要它：代理工具会改核心状态（建会话 / 派发），而模型循环跑在工作线程上——
///  状态所有权不变（只有核心线程写），这里只做“发一条命令并等回包”。
///  `mpsc::Sender` 不是 `Sync`，所以用一把锁串行化入队；入队本身不参与任何业务判定。
pub struct ProxyBridge {
    handle: std::sync::Mutex<Handle>,
}

impl ProxyBridge {
    pub fn new(handle: Handle) -> ProxyBridge {
        ProxyBridge {
            handle: std::sync::Mutex::new(handle),
        }
    }

    fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Core) -> Result<T, String> + Send + 'static,
    {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        handle.call(f)
    }
}

impl crate::capabilities::conversation::proxy::api::ProxyHost for ProxyBridge {
    fn catalog(&self, scope: d::CatalogScope) -> Result<d::Catalog, String> {
        self.call(move |core| core.proxy_catalog(scope))
    }

    fn create_session(&self, spec: &d::NewSession) -> Result<d::Created, String> {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        // 模型侧只是**适配器**：调用者身份 + 载荷交给唯一实现（建会话 + 建好就开工）。
        let caller = crate::capabilities::runtime::api::Caller::Role {
            role: "core_proxy".to_string(),
            work: spec.parent.clone().unwrap_or_default(),
            agent: SPEAKER.to_string(),
        };
        handle.create_session_call(&caller, spec)
    }

    fn send(&self, target: &str, msg: &d::Relayed) -> Result<(), String> {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        let target = target.to_string();
        // 运行态闸门 + 形态：**一次核心命令**问清（无此会话 / 已暂停 / 已关闭都如实拒绝，
        // 不给模型一个"发出去了其实没跑"的假回执）。
        let mode = handle.call({
            let t = target.clone();
            move |core| core.turn.dispatch_target(&t)
        })?;
        // 先记来源：这条不是用户原话这个事实要进目标会话的可回放记录（正文不伪装成用户发言）。
        handle.record_notice(&target, SessionEvent::Notice(d::relay_note(msg.kind)));
        if mode == "collab" {
            // 协作子会话：它此刻挂着哪张卡时，**只有当这次转达明确写了选项 id 才代答**（附言 = 转达的话）；
            // 没写选项 id 就没有答案——不替用户猜（见 docs/session/session-model.md 的「请用户裁决」）。
            let card = handle.call({
                let t = target.clone();
                move |core| {
                    core.turn
                        .open_queue(&t, core.registry.as_ref(), core.probe.as_ref())
                }
            })?;
            match (card, msg.reply.as_deref()) {
                (Some(q), Some(reply)) => {
                    let option = d::relay_reply(&q.card.options, reply)?;
                    crate::capabilities::turn::turn::api::spawn_detached_collab_answer(
                        &*handle, &target, &q.card.id, &option, &msg.text,
                    );
                }
                // 挂着卡但这次转达不是代答：只记来源、不替用户作答（那一问按它自己声明的收场走）。
                (Some(q), None) => handle.record_notice(
                    &target,
                    SessionEvent::Notice(format!(
                        "[代理] 这条子会话正等你裁决（卡 {}）：这次转达没有带选项 id，不作代答。",
                        q.card.id
                    )),
                ),
                (None, _) => {
                    crate::capabilities::turn::turn::api::spawn_detached_collab(&*handle, &target)
                }
            }
            return Ok(());
        }
        // 单 agent：以"核心派的活"注入（派发行是核心自己的行，**不冒充用户原话**），
        // 并**脱离调用方点火**——代理不等它跑完，靠 observe / messages 回头看。
        crate::capabilities::turn::turn::api::spawn_detached_node(&*handle, &target, &msg.text);
        Ok(())
    }

    fn observe(
        &self,
        session: &str,
        view: d::ObserveView,
        since: Option<&str>,
    ) -> Result<d::Snapshot, String> {
        let session = session.to_string();
        let since = since.map(|s| s.to_string());
        self.call(move |core| core.proxy_observe(&session, view, since.as_deref()))
    }

    fn control(
        &self,
        session: &str,
        action: d::ControlAction,
        reason: &str,
    ) -> Result<d::ControlState, String> {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        // 模型侧只是**适配器**：调用者身份 + 控制语义交给唯一实现。
        let caller = crate::capabilities::runtime::api::Caller::Role {
            role: "core_proxy".to_string(),
            work: session.to_string(),
            agent: SPEAKER.to_string(),
        };
        let state = handle.control_session_call(&caller, session, action, reason)?;
        Ok(d::ControlState {
            session: session.to_string(),
            action,
            state,
        })
    }

    fn messages(
        &self,
        session: &str,
        from: usize,
        count: usize,
    ) -> Result<d::MessagesPage, String> {
        let session = session.to_string();
        self.call(move |core| core.proxy_messages(&session, from, count))
    }
}

impl Handle {
    /// 目的：代理会话——把代理工具的成员侧执行面（`ProxyHandler`）装进这一回合的工具环境。
    ///   执行经队列桥回到核心线程，核心状态的所有权不变；非代理会话、或已装过，原样交回。
    pub(super) fn inject_proxy_handler(
        &self,
        mut session: crate::capabilities::conversation::session::api::AgentSession,
        sid: &str,
    ) -> Result<crate::capabilities::conversation::session::api::AgentSession, String> {
        use crate::capabilities::conversation::proxy::api as dp;
        let wanted = session.tools.as_ref().is_some_and(|tools| {
            tools.handlers.is_empty() && tools.face.allowed.iter().any(|t| dp::is_proxy_tool(t))
        });
        if !wanted {
            return Ok(session);
        }
        let face = match session.tools.as_ref() {
            Some(tools) => tools.face.allowed.clone(),
            None => Vec::new(),
        };
        let granted = {
            let sid = sid.to_string();
            self.call(move |core| {
                Ok(core
                    .lifecycle
                    .history_open(&sid)?
                    .0
                    .delegation
                    .map(|d| d.granted_at))
            })?
        };
        let grant = granted.map(|_| dp::Grant {
            tools: face,
            ..Default::default()
        });
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let host: std::sync::Arc<
            dyn crate::capabilities::conversation::proxy::api::ProxyHost + Send + Sync,
        > = std::sync::Arc::new(crate::capabilities::runtime::service::ProxyBridge::new(
            self.clone(),
        ));
        let handler = crate::capabilities::conversation::proxy::api::ProxyHandler::new(
            host,
            self.book.clone(),
            dp::ProxyCall {
                grant,
                parent: Some(sid.to_string()),
                now,
            },
        );
        if let Some(tools) = session.tools.as_mut() {
            tools.handlers = vec![std::sync::Arc::new(handler)];
        }
        Ok(session)
    }
}
