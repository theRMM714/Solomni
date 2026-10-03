//! 核心代理工具的**执行面**：按声明校验参数 → 判授权 → 查幂等账 → 把动作交给 ProxyHost。
//!
//! 工具逻辑只到这里：它不碰会话机制。宿主是 `ProxyBridge`（队列桥：把 ProxyHost 的每个方法
//! 交给核心线程执行）与 `Conductor` 的 `proxy_*` 方法族；代理会话那一回合挂的是 `ProxyHandler`。

use crate::capabilities::conductor::domain::proxy as d;
use crate::capabilities::conductor::ports::ProxyHost;
use crate::capabilities::prompt::api::ToolTexts;
use crate::capabilities::tools::api::{arg_fault_text, ToolBook, ToolOutcome};
use std::collections::BTreeMap;
use std::sync::Arc;

/// 代理工具的执行器：持有宿主端口、工具声明书、回执文案与幂等账本。
pub struct ProxyTools {
    host: Arc<dyn ProxyHost + Send + Sync>,
    book: ToolBook,
    texts: Arc<ToolTexts>,
    /// 幂等账本：工具/request_id → 上一次的结果（重放直接回同一结果，不再动宿主）。
    done: BTreeMap<String, ToolOutcome>,
}

impl ProxyTools {
    pub fn new(
        host: Arc<dyn ProxyHost + Send + Sync>,
        book: ToolBook,
        texts: Arc<ToolTexts>,
    ) -> ProxyTools {
        ProxyTools {
            host,
            book,
            texts,
            done: BTreeMap::new(),
        }
    }

    /// 执行一次代理工具调用：name 必须是五个代理工具之一；args_json 是模型给的参数。
    /// 参数先按总表声明校验（形状只有一份真相），再做语义校验、授权与宿主调用。
    pub fn call(&mut self, ctx: &d::ProxyCall, name: &str, args_json: &str) -> ToolOutcome {
        if !d::is_proxy_tool(name) {
            return deny(format!("不是代理工具：{}", name));
        }
        let Some(schema) = self.book.get(name) else {
            return deny(format!("工具总表里没有这个工具：{}", name));
        };
        let value: serde_json::Value = match serde_json::from_str(args_json) {
            Ok(v) => v,
            Err(e) => return deny(format!("{} 的参数不是合法 JSON：{}", name, e)),
        };
        if let Err(fault) = schema.check(&value) {
            return deny(arg_fault_text(&self.texts, name, schema, &fault));
        }
        let mut value = value;
        schema.apply_defaults(&mut value);
        if name == d::CATALOG {
            self.catalog(ctx, &value)
        } else if name == d::CREATE {
            self.create(ctx, &value)
        } else if name == d::SEND {
            self.send(ctx, &value)
        } else if name == d::OBSERVE {
            self.observe(ctx, &value)
        } else if name == d::MESSAGES {
            self.messages(ctx, &value)
        } else {
            self.control(ctx, &value)
        }
    }

    fn catalog(&self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::CatalogArgs = match parse_arg(d::CATALOG, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let scope = match d::catalog_scope(&args) {
            Ok(s) => s,
            Err(e) => return deny(e),
        };
        if let Err(e) = d::authorize(ctx, d::CATALOG, None, None) {
            return deny(e);
        }
        match self.host.catalog(scope) {
            Ok(c) => match d::render_catalog(scope, &c) {
                Ok(text) => ok(text),
                Err(e) => deny(e),
            },
            Err(e) => deny(e),
        }
    }

    fn create(&mut self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::CreateArgs = match parse_arg(d::CREATE, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        if args.request_id.trim().is_empty() {
            return deny("request_id 不能为空（幂等标识）".to_string());
        }
        let key = idem_key(d::CREATE, &args.request_id);
        if let Some(prev) = self.done.get(&key) {
            return prev.clone();
        }
        if let Err(e) = d::authorize(ctx, d::CREATE, None, None) {
            return deny(e);
        }
        // 校验要对着登记处与模块清单做：核不过就整条拒绝，**不调用 create**（不留半成品）。
        let catalog = match self.host.catalog(d::CatalogScope::All) {
            Ok(c) => c,
            Err(e) => return deny(e),
        };
        let mut spec = match d::resolve_new_session(&args, &catalog) {
            Ok(s) => s,
            Err(e) => return deny(e),
        };
        // 父会话由机制从调用上下文填，不接受模型自参。
        spec.parent = ctx.parent.clone();
        for a in &spec.agents {
            if let Some(m) = &a.model {
                if let Err(e) = d::authorize(ctx, d::CREATE, Some(m), None) {
                    return deny(e);
                }
            }
        }
        match self.host.create_session(&spec) {
            Ok(created) => {
                let out = json_ok(&created);
                self.done.insert(key, out.clone());
                out
            }
            // 创建失败**不记账**：重放会再试一次，而不是把失败当成已完成。
            Err(e) => deny(e),
        }
    }

    fn send(&mut self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::SendArgs = match parse_arg(d::SEND, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        if args.request_id.trim().is_empty() {
            return deny("request_id 不能为空（幂等标识）".to_string());
        }
        let key = idem_key(d::SEND, &args.request_id);
        if let Some(prev) = self.done.get(&key) {
            return prev.clone();
        }
        if let Err(e) = d::authorize(ctx, d::SEND, None, None) {
            return deny(e);
        }
        let (targets, relayed) = match d::relay(&args, ctx) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        for t in &targets {
            if let Err(e) = d::authorize(ctx, d::SEND, None, Some(t)) {
                return deny(e);
            }
        }
        // 多目标：逐个记录成功与失败，部分成功照实回报，不产生未记录的隐式发送。
        let mut sent: Vec<String> = Vec::new();
        let mut failed: Vec<serde_json::Value> = Vec::new();
        for t in &targets {
            match self.host.send(t, &relayed) {
                Ok(()) => sent.push(t.clone()),
                Err(e) => failed.push(serde_json::json!({ "target": t, "why": e })),
            }
        }
        let out = ToolOutcome {
            ok: failed.is_empty(),
            output: serde_json::json!({
                "source": relayed.source.as_str(),
                "sent": sent,
                "failed": failed,
            })
            .to_string(),
        };
        self.done.insert(key, out.clone());
        out
    }

    fn observe(&self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::ObserveArgs = match parse_arg(d::OBSERVE, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let (session, view, since) = match d::observe_request(&args) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        if let Err(e) = d::authorize(ctx, d::OBSERVE, None, Some(&session)) {
            return deny(e);
        }
        match self.host.observe(&session, view, since.as_deref()) {
            Ok(s) => json_ok(&s),
            Err(e) => deny(e),
        }
    }

    /// 消息倒查：只读，不改任何状态，也不进幂等账。
    fn messages(&self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::MessagesArgs = match parse_arg(d::MESSAGES, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let (session, from, count) = match d::messages_request(&args) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        if let Err(e) = d::authorize(ctx, d::MESSAGES, None, Some(&session)) {
            return deny(e);
        }
        match self.host.messages(&session, from, count) {
            Ok(page) => json_ok(&page),
            Err(e) => deny(e),
        }
    }

    fn control(&mut self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::ControlArgs = match parse_arg(d::CONTROL, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let (session, action, reason, request_id) = match d::control_request(&args) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        let key = idem_key(d::CONTROL, &request_id);
        if let Some(prev) = self.done.get(&key) {
            return prev.clone();
        }
        if let Err(e) = d::authorize(ctx, d::CONTROL, None, Some(&session)) {
            return deny(e);
        }
        match self.host.control(&session, action, &reason) {
            Ok(state) => {
                let out = json_ok(&state);
                self.done.insert(key, out.clone());
                out
            }
            Err(e) => deny(e),
        }
    }
}

fn parse_arg<T: serde::de::DeserializeOwned>(
    name: &str,
    value: &serde_json::Value,
) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("{} 的参数形状不对：{}", name, e))
}

fn idem_key(tool: &str, request_id: &str) -> String {
    format!("{}/{}", tool, request_id.trim())
}

fn ok(text: String) -> ToolOutcome {
    ToolOutcome {
        ok: true,
        output: text,
    }
}

fn deny(text: String) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: text,
    }
}

fn json_ok<T: serde::Serialize>(v: &T) -> ToolOutcome {
    match serde_json::to_string(v) {
        Ok(s) => ok(s),
        Err(e) => deny(format!("回执序列化失败：{}", e)),
    }
}
// ---------- 真实宿主：Conductor 的代理方法族 + 队列桥 ----------

use super::{now_ts, validate_work_name, Conductor, Session};
use crate::capabilities::conductor::api::{
    AgentInstance, CollabStep, ConductorHandle, SessionOps, WorkMode, WorkSpec,
};
use crate::capabilities::session::api::{
    AgentSession, Delegation, RunState, SessionEvent, SessionMeta, SessionParams,
};
use crate::capabilities::workspace::api::{ExecSpec, Sandbox};

fn tool_mode_str(m: crate::capabilities::llm::api::ToolMode) -> String {
    match m {
        crate::capabilities::llm::api::ToolMode::Native => "native".to_string(),
        crate::capabilities::llm::api::ToolMode::Envelope => "envelope".to_string(),
    }
}

impl Conductor {
    /// 一个会话的转录行（回放口径）：观察计数与消息倒查共用，避免两处各解析一遍。
    fn proxy_lines(
        &self,
        sid: &str,
    ) -> Result<Vec<crate::capabilities::session::api::LineView>, String> {
        let (_, events) = self.history_open(sid)?;
        let mut out = Vec::new();
        for ev in &events {
            if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
                continue;
            }
            let Some(arr) = ev.get("lines").and_then(|l| l.as_array()) else {
                continue;
            };
            for l in arr {
                if let Ok(v) =
                    serde_json::from_value::<crate::capabilities::session::api::LineView>(l.clone())
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

    /// 代理工具：只读清单（agent / 模块 / 模型），只回公开事实，不碰密钥与私有路径。
    pub fn proxy_catalog(&self, scope: d::CatalogScope) -> Result<d::Catalog, String> {
        let agents = if scope.covers_agents() {
            self.registry()
                .agent_views()
                .into_iter()
                .map(|v| d::AgentFact {
                    name: v.name,
                    modules: v.modules,
                    model: v.model,
                    note: v.note,
                })
                .collect()
        } else {
            Vec::new()
        };
        let modules = if scope.covers_modules() {
            self.scan()
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

    /// 代理工具：观察（元信息；**不回消息正文**）。
    pub fn proxy_observe(
        &self,
        sid: &str,
        view: d::ObserveView,
        _since: Option<&str>,
    ) -> Result<d::Snapshot, String> {
        // 存在性与运行态都取 meta（**不回放流水**）；"此刻在不在跑"再看会话中心。
        let meta = self
            .history
            .meta(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let list = self.history_list();
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
        Ok(d::Snapshot {
            session: sid.to_string(),
            state: state.to_string(),
            pending,
            artifacts,
            message_count,
            cursor: message_count.map(|n| n.to_string()),
        })
    }

    /// 代理工具：消息倒查（0 = 最新一条，按新→旧）。
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

    /// 一个会话的形态（single / collab）：落盘 meta 是唯一真相；取不到就回空串。
    pub(crate) fn session_mode_str(&self, sid: &str) -> String {
        self.history_open(sid)
            .map(|(m, _)| m.mode)
            .unwrap_or_default()
    }

    /// 代理工具：建一个**子工作**（single / collab）：编排归属是父会话，
    /// 工作区与沙箱是它自己的（`own_work`）。
    pub fn proxy_create(&mut self, spec: &d::NewSession) -> Result<d::Created, String> {
        let parent = spec
            .parent
            .clone()
            .ok_or_else(|| "代理建会话缺少父会话：它由机制提供，不接受模型自参".to_string())?;
        let (pmeta, _) = self.history_open(&parent)?;
        let base = spec
            .workspace
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("w-{}", spec.request_id));
        let name = self.unique_work_name(&format!("{}--{}", parent, base), "work");
        let mode = match spec.mode {
            d::SessionMode::Single => WorkMode::Single,
            d::SessionMode::Multi => WorkMode::Collab,
        };
        let agents: Vec<AgentInstance> = spec
            .agents
            .iter()
            .map(|a| AgentInstance {
                name: a.name.clone(),
                transient: a.transient,
                modules: a.modules.clone(),
                model: a.model.clone(),
            })
            .collect();
        // 多 agent 是协作工作：把各 agent 的 objective 合成"本次需求"（协作必须有需求）。
        // 代理形态不会出现在这里：代理工具只能建 single / multi（代理不能往里套代理）。
        let task = if mode == WorkMode::Collab {
            Some(
                spec.agents
                    .iter()
                    .map(|a| format!("{}：{}", a.name, a.objective))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        } else {
            None
        };
        let work = WorkSpec {
            name,
            mode,
            agents,
            task,
            delegate: false,
            tier: pmeta.exec.tier,
        };
        let opened = self.create_work_inner(work, Some(&parent), true)?;
        Ok(d::Created {
            session: opened.sid,
            agents: opened.agents,
        })
    }

    /// 代理工具：控制（**只做运行态的状态转移**，不启动也不停止生成——停止要 JobRegistry，
    /// 只有句柄拿得到，见 `ProxyBridge::control`）。可回放记录也由句柄那一侧写（它才推得到事件台）。
    /// - pause：冻上（此后的派发与唤醒一律拒绝）；在跑的那次由句柄停；
    /// - resume：解冻；随后由句柄按形态唤醒它接着走；
    /// - close：终态。整棵子树里还有在生成的就拒绝（先 stop / pause），不留半个状态；
    /// - stop：级联停止由句柄承担，不进这条。
    pub fn proxy_control(
        &self,
        sid: &str,
        action: d::ControlAction,
    ) -> Result<d::ControlState, String> {
        let meta = self.history.meta(sid)?;
        let target = meta.name.clone();
        let set = |run: RunState| -> Result<(), String> {
            let mut m = meta.clone();
            m.run = run;
            self.history.save_meta(&m)
        };
        // 先做状态转移（只改落盘运行态），回执状态在下面按动作给。
        match action {
            d::ControlAction::Stop => {
                return Err("stop 不走这条：级联停止由句柄承担".to_string());
            }
            d::ControlAction::Pause => match meta.run {
                RunState::Closed => {
                    return Err(format!("会话 {} 已关闭：不能再暂停", target));
                }
                RunState::Paused => {}
                RunState::Active => set(RunState::Paused)?,
            },
            d::ControlAction::Resume => match meta.run {
                RunState::Closed => {
                    return Err(format!("会话 {} 已关闭：终态，不能 resume", target));
                }
                RunState::Paused => set(RunState::Active)?,
                RunState::Active => {}
            },
            d::ControlAction::Close => {
                // “关闭已完成或已停止的会话”：子树里还有在生成的就整条拒绝。
                for s in self.subtree_of(sid) {
                    if self.running.contains(&s) {
                        return Err(Self::running_refusal(&s));
                    }
                }
                if meta.run != RunState::Closed {
                    set(RunState::Closed)?;
                }
            }
        };
        let state = match action {
            d::ControlAction::Pause => "paused",
            d::ControlAction::Resume => "active",
            d::ControlAction::Close => "closed",
            d::ControlAction::Stop => "stopped",
        };
        Ok(d::ControlState {
            session: target,
            action,
            state: state.to_string(),
        })
    }

    /// 建一个**代理会话**（`mode="proxy"` + 全权委托）：核心在这里跟用户对话，用代理工具代他决定。
    /// 没有 agent、没有模块工具；这一回合的工具面是 `core_proxy`（六项代理工具 + 只读核实）。
    /// 测试便捷入口：只要引用（产品入口经 `create_work(WorkMode::Proxy)`）。
    #[cfg(test)]
    pub fn create_proxy(&mut self, name: &str, granted_at: i64) -> Result<String, String> {
        self.create_proxy_with_facts(name, granted_at)
            .map(|(sid, _)| sid)
    }

    /// 建代理会话的真身：返回（会话引用, **开场事实**）——事实交给 api 层发布到事件台
    /// （核心不持有事件台；与 `create_work` 同一条口径）。
    pub(crate) fn create_proxy_with_facts(
        &mut self,
        name: &str,
        granted_at: i64,
    ) -> Result<(String, Vec<SessionEvent>), String> {
        validate_work_name(name)?;
        if self.sessions.contains_key(name) || self.history.load(name).is_ok() {
            return Err(format!("工作名已存在：{}", name));
        }
        let meta = SessionMeta {
            name: name.to_string(),
            mode: "proxy".to_string(),
            delegate: false,
            modules: Vec::new(),
            task: None,
            ts: now_ts(),
            agents: Vec::new(),
            exec: ExecSpec::default(),
            parent: None,
            node: None,
            delegation: Some(Delegation { granted_at }),
            own_work: false,
            run: RunState::Active,
        };
        self.workspace.prepare(name, &[])?;
        let session = self.build_proxy(&meta)?;
        let mut events = session.open();
        self.history.create(&meta)?;
        self.sessions
            .insert(name.to_string(), Session::Single(session));
        self.record_events(name, &mut events);
        Ok((name.to_string(), events))
    }

    /// 装配一个代理会话对象（创建与重建**共用同一处**，两处不各拼一遍）。
    pub(crate) fn build_proxy(&self, meta: &SessionMeta) -> Result<AgentSession, String> {
        let roots = self.workspace.roots(&meta.name, &[])?;
        let sb = Sandbox {
            work_name: meta.name.clone(),
            agent: d::SPEAKER.to_string(),
            shared: roots.shared.clone(),
            private: roots.shared.clone(),
            modules: BTreeMap::new(),
            texts: self.prompt.tools(),
        };
        let channel = self.registry.core_channel();
        let tool_mode = if channel.is_some() {
            self.registry.tool_mode(None)
        } else {
            crate::capabilities::llm::api::ToolMode::Envelope
        };
        let (chat, demo) = self.llm.core_channel(channel.as_ref());
        let note = if demo {
            Some("（无可用模型：本次走演示通道，不会原生调用工具）".to_string())
        } else {
            None
        };
        // 身份是**角色提示词**（core_proxy），不是某个 agent 的模块能力包：
        // 机制 / 环境 / 调用约定仍与 agent 身份同一份口径（见 workspace 的 render_system）。
        let mut params = SessionParams::from_workspace(d::SPEAKER, &sb, &[]);
        params.role_system = Some(crate::capabilities::prompt::api::Segment::ProxySystem);
        let tools = self.tools_env(&[], &sb, BTreeMap::new(), false, tool_mode, "core_proxy");
        let mut s = AgentSession::new(
            d::SPEAKER,
            params,
            chat,
            note,
            Some(tools),
            self.prompt.refs(),
            self.prompt.tools(),
        );
        s.set_compact_budget(self.compact_budget(None));
        Ok(s)
    }
}

/// 代理工具的**队列宿主**：把 ProxyHost 的每个方法交给核心线程执行。
///
/// 为什么需要它：代理工具会改核心状态（建会话 / 派发），而模型循环跑在工作线程上——
/// 状态所有权不变（只有核心线程写），这里只做“发一条命令并等回包”。
/// `mpsc::Sender` 不是 `Sync`，所以用一把锁串行化入队；入队本身不参与任何业务判定。
pub struct ProxyBridge {
    handle: std::sync::Mutex<ConductorHandle>,
}

impl ProxyBridge {
    pub fn new(handle: ConductorHandle) -> ProxyBridge {
        ProxyBridge {
            handle: std::sync::Mutex::new(handle),
        }
    }

    fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Conductor) -> Result<T, String> + Send + 'static,
    {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        handle.call(f)
    }
}

impl ProxyHost for ProxyBridge {
    fn catalog(&self, scope: d::CatalogScope) -> Result<d::Catalog, String> {
        self.call(move |core| core.proxy_catalog(scope))
    }

    fn create_session(&self, spec: &d::NewSession) -> Result<d::Created, String> {
        let spec = spec.clone();
        self.call(move |core| core.proxy_create(&spec))
    }

    fn send(&self, target: &str, msg: &d::Relayed) -> Result<(), String> {
        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        let target = target.to_string();
        // 运行态闸门 + 形态：**一次核心命令**问清（无此会话 / 已暂停 / 已关闭都如实拒绝，
        // 不给模型一个"发出去了其实没跑"的假回执）。
        let mode = handle.call({
            let t = target.clone();
            move |core| core.dispatch_target(&t)
        })?;
        // 先记来源：这条不是用户原话这个事实要进目标会话的可回放记录（正文不伪装成用户发言）。
        handle.record_notice(&target, SessionEvent::Notice(d::relay_note(msg)));
        if mode == "collab" {
            // 协作子会话：**按它此刻等的是哪一关**落到对应的阶段步（与前端读 pending 同一口径）。
            // 它自己的核心 AI 判明确性；这里只负责把话送到正确的门上。
            let pending = handle.call({
                let t = target.clone();
                move |core| core.collab_pending(&t)
            })?;
            match d::gate_route(pending.as_ref().map(|p| p.decision_parts().0)) {
                // 代拟名单这关是二选一，且是**短步骤**（不跑泵）：在核心线程上直接推进。
                d::GateRoute::ConfirmSlate => {
                    handle.collab_step(&target, CollabStep::ConfirmSlate, &msg.text)?;
                }
                d::GateRoute::Begin => {
                    handle.spawn_detached_collab_step(&target, CollabStep::Begin, &msg.text);
                }
                d::GateRoute::Decide => {
                    handle.spawn_detached_collab_step(&target, CollabStep::Decide, &msg.text);
                }
                d::GateRoute::Resume => handle.spawn_detached_collab(&target),
            }
            return Ok(());
        }
        // 单 agent：以"核心派的活"注入（派发行是核心自己的行，**不冒充用户原话**），
        // 并**脱离调用方点火**——代理不等它跑完，靠 observe / messages 回头看。
        handle.spawn_detached_node(&target, &msg.text);
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
        let session = session.to_string();
        if action == d::ControlAction::Stop {
            // 级联停止：整棵子树（本会话 + 所有后代）正在跑的生成都立即中断。
            let stopped = handle.stop_tree(&session)?;
            handle.record_notice(
                &session,
                SessionEvent::Notice(d::control_note(action, reason)),
            );
            return Ok(d::ControlState {
                session,
                action,
                state: format!("stopped:{}", stopped.len()),
            });
        }
        // 运行态转移在核心线程上（落盘 meta 是唯一真相）；启动 / 停止生成只有句柄做得了。
        let state = handle.call({
            let s = session.clone();
            move |core| core.proxy_control(&s, action)
        })?;
        match action {
            // 先冻上运行态，再停生成：此刻起任何派发与唤醒都被拒（顺序不能反）。
            d::ControlAction::Pause => {
                let _ = handle.stop_tree(&session);
            }
            // 解冻后按形态唤醒它接着走：协作继续推进链，代理自己再想一轮，单 agent 接着说完。
            d::ControlAction::Resume => {
                let mode = handle.call({
                    let s = session.clone();
                    move |core| core.dispatch_target(&s)
                })?;
                match mode.as_str() {
                    "collab" => handle.spawn_detached_collab(&session),
                    "proxy" => handle.spawn_detached_proxy(&session),
                    _ => handle.spawn_detached_continue(&session),
                }
            }
            _ => {}
        }
        handle.record_notice(
            &session,
            SessionEvent::Notice(d::control_note(action, reason)),
        );
        Ok(state)
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
/// 代理工具的**成员侧执行者**：把 `core_proxy` 会话那一回合里的代理工具调用接到 `ProxyTools` 上。
///
/// 装配进代理会话的 `MemberTools.handlers` 之后，代理会话就是一个**普通成员会话**——
/// `converse_with` / 转录 / 回档 / 压缩 / 停止全部复用，循环里没有任何“代理专用”分支。
/// 执行经队列桥回核心线程（`ProxyHost`），核心状态的所有权不变。
pub struct ProxyHandler {
    tools: std::sync::Mutex<ProxyTools>,
    ctx: d::ProxyCall,
}

impl ProxyHandler {
    pub fn new(
        host: Arc<dyn ProxyHost + Send + Sync>,
        book: ToolBook,
        texts: Arc<ToolTexts>,
        ctx: d::ProxyCall,
    ) -> ProxyHandler {
        ProxyHandler {
            tools: std::sync::Mutex::new(ProxyTools::new(host, book, texts)),
            ctx,
        }
    }
}

impl crate::kernel::ports::ToolHandler for ProxyHandler {
    fn owns(&self, name: &str) -> bool {
        d::is_proxy_tool(name)
    }

    fn run(&self, _session: &str, name: &str, args_json: &str) -> ToolOutcome {
        let mut tools = self.tools.lock().unwrap_or_else(|e| e.into_inner());
        tools.call(&self.ctx, name, args_json)
    }
}
