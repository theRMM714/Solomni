//! 目的：单 agent 的回合收发——工具形态刷新、生成前准备、节点派发准备、同步生成入口、继续；
//!   以及会话的放取（单 agent / 流程槽）与交回时的收尾判据。
//! 管：一次单 agent 回合的判据（形态变了先提示、继续时末条必须是用户发言）、放取时的运行态闸门、
//!   交回时按落盘运行态作废裁决队、压缩回合的身份块、系统消息注入、回合编号。
//! 不管：拟名单、会话启用、协作链推进（在 `flow`）、模型调用本身（在 `collab` 与 `llm`）。
//! 联动：宿主的工作线程经 `turn::api::generation` 取准备结果；登记处与主机探针由调用方按参数给（本层不持登记处）。

use std::sync::Arc;

use crate::kernel::ports::HostProbe;
use crate::units::catalog::registry::api::Registry;
use crate::units::conversation::session::api::{
    AgentSession, Flow, Gates, RunState, seat as seat_of, subtree_of,
};
use crate::units::transcript::api::SessionEvent;
use crate::units::turn::turn::api::{Prepared, Session};

use super::flow::NodeSpawn;
use super::session::Turn;

/// 目的：单 agent 会话轮不到用户发言时的提醒（"继续"与"发言"两个入口共用这一句）。
pub(crate) const NEED_USER: &str =
    "最后一条是 AI 发言：部分供应商不允许连续 AI 发言；请先发言再继续。";

impl Turn {
    /// 目的：形态**不钉在会话里**：每次生成前按登记处重新解析。
    ///  变了 → 只改会话参数里的那一格并给用户一句通知（身份块下次调用就按新约定渲染）；没变 → 什么都不做。
    ///  这样"用户改了登记处就重新查、没改就不管"，同时身份块里的约定与实际协议始终一致。
    pub(crate) fn refresh_tool_mode(
        &mut self,
        sid: &str,
        registry: &dyn Registry,
    ) -> Result<Option<String>, String> {
        let a = match self.session(sid) {
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
        registry
            .channel(a.model.as_deref())
            .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
        let want = registry.tool_mode(a.model.as_deref());
        let cur = match self.session(sid) {
            Some(Session::Single(s)) => s.tool_mode(),
            // 还没装进内存的会话：交给 ensure_session 按当前形态建，这里不动
            _ => want,
        };
        if cur == want {
            return Ok(None);
        }
        // **只改这一格**：形态是登记处派生出来的参数，没必要把整个会话从盘上重建一遍
        //（身份块每次调用现渲染，改完这一格下一回合就生效）。
        if let Some(Session::Single(s)) = self.session_mut(sid) {
            s.set_tool_mode(want);
        }
        Ok(Some(match want {
            crate::units::llm::api::ToolMode::Native => {
                "工具调用形态已按登记处改为**原生工具调用**（本条起生效）".to_string()
            }
            crate::units::llm::api::ToolMode::Envelope => {
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
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<Prepared, String> {
        // 还没装进内存的会话先从落盘重建（"继续"可能先于"打开"到达；真没这个会话仍然报无此会话）。
        self.ensure_session(sid, registry, probe)?;
        if matches!(self.session(sid), Some(Session::Flow(_))) {
            return Ok(Prepared::NotSingle);
        }
        let mut prefix: Vec<SessionEvent> = Vec::new();
        if let Some(n) = self.refresh_tool_mode(sid, registry)? {
            prefix.push(SessionEvent::Notice(n));
        }
        if text.is_none() {
            let last_is_user =
                matches!(self.session(sid), Some(Session::Single(s)) if s.last_is_user());
            if !last_is_user {
                prefix.push(SessionEvent::Notice(NEED_USER.to_string()));
                return Ok(Prepared::Immediate(prefix));
            }
        }
        // 思考强度按**工作根**解析（子会话共享）；模型没登记该档会在这里如实报错。
        let llm = self.llm_opts_for(registry, sid, want_stream)?;
        // 身份块**现渲染**（形态刚在上一步对齐过，所以这里读到的就是本回合的形态）。
        let identity = match self.session(sid) {
            Some(Session::Single(s)) => s.params().identity(&*self.prompt, s.tool_mode()),
            _ => return Ok(Prepared::NotSingle),
        };
        let mut session = self.take_single(sid)?;
        self.refresh_context_budget(registry, &mut session);
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
        seat: Option<&str>,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<Prepared, String> {
        self.ensure_session(child, registry, probe)?;
        if matches!(self.session(child), Some(Session::Flow(_))) {
            return Ok(Prepared::NotSingle);
        }
        // 节点的生成跟随设置里的流式开关；思考强度按工作根解析（子会话共享）。
        let llm = self.llm_opts_for(registry, child, true)?;
        let mut prefix: Vec<SessionEvent> = Vec::new();
        if let Some(n) = self.refresh_tool_mode(child, registry)? {
            prefix.push(SessionEvent::Notice(n));
        }
        let mut session = self.take_single(child)?;
        self.refresh_context_budget(registry, &mut session);
        session.set_thinking(llm.thinking.clone());
        // 席位是**回合的值**：节点要坐的那一席由流程给（None = 会话自己的静止席位）。
        // 先换席位再渲染身份——身份块按席位现取（与工具面同源），反过来会拿上一席的身份干活。
        let seat = seat
            .map(str::to_string)
            .unwrap_or_else(|| session.seat().to_string());
        if seat_of(&seat).is_none() {
            return Err(format!("会话 {} 的席位没登记：{}", child, seat));
        }
        session.enter_seat(&seat);
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
    ///  生产路径的生成在工作线程上（见 `turn::api::generation`）。
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn single_say(
        &mut self,
        sid: &str,
        text: &str,
        io: &crate::units::conversation::session::api::TurnIo,
        cancel: &std::sync::atomic::AtomicBool,
        emit: &mut dyn FnMut(SessionEvent),
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<Vec<SessionEvent>, String> {
        match self.prepare_single(sid, Some(text), io.llm.stream, registry, probe)? {
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
                    session.say(text, &identity, io, cancel, emit, &mut sink);
                }
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    self.log.warn("turn::single_say", "生成被用户中止");
                }
                self.put_single_recorded(sid, session, &events);
                Ok(events)
            }
        }
    }

    /// 目的：继续：由用户点击授权。单 agent 会话需要轮到用户（末条是 AI 就只提醒、不发请求）；
    ///  协作不需要用户发言，继续 = 从断点推进流水线。
    ///  **测试用同步入口**：生产路径的两条（单 agent / 协作）都在工作线程上跑（见 `turn::api::generation`）。
    #[cfg(test)]
    pub fn continue_flow(
        &mut self,
        sid: &str,
        io: &crate::units::conversation::session::api::TurnIo,
        cancel: &std::sync::atomic::AtomicBool,
        emit: &mut dyn FnMut(SessionEvent),
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<Vec<SessionEvent>, String> {
        self.ensure_session(sid, registry, probe)?;
        // 身份块要读提示词册：先取一份句柄，免得与会话的可变借用撞车。
        let prompt = Arc::clone(&self.prompt);
        let mut events = {
            let s = self.session_mut(sid).ok_or("无此活动会话")?;
            match s {
                Session::Single(s) => {
                    if s.last_is_user() {
                        let mut out = Vec::new();
                        {
                            let mut sink = |ev: SessionEvent| out.push(ev);
                            let identity = s.params().identity(&*prompt, s.tool_mode());
                            s.continue_reply(&identity, io, cancel, emit, &mut sink);
                        }
                        out
                    } else {
                        vec![SessionEvent::Notice(NEED_USER.to_string())]
                    }
                }
                // 流程不需要用户发言：从断点推进流水线。
                Session::Flow(m) => {
                    let mut out = Vec::new();
                    m.resume(&mut |e| out.push(e));
                    out
                }
            }
        };
        self.record_events(sid, &mut events);
        Ok(events)
    }

    /// 目的：把单 agent 会话**交给工作线程**（核心表里留"生成中"）。
    ///  取出的窗口内，核心队列是空的——读接口与其它会话的命令因此照常。
    pub(crate) fn take_single(&mut self, sid: &str) -> Result<AgentSession, String> {
        // 运行态闸门：暂停 / 关闭的会话不启动任何生成（唤醒与派发都从这里过）。
        self.dispatch_gate(sid)?;
        if self.is_running(sid) {
            return Err(Self::running_refusal(sid));
        }
        match self.take(sid) {
            Some(Session::Single(s)) => {
                self.mark_running(sid.to_string());
                Ok(s)
            }
            Some(other) => {
                // 不是单 agent：原样放回，交给协作那条路（B-1 只搬单 agent）。
                self.put(sid.to_string(), other);
                Err("该会话不是单 agent 模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：把一个流程会话**交给工作线程**（核心表里留"生成中"）。
    ///  会话还没装进内存时先从落盘重建：流程的"继续"可能先于"打开"到达。
    pub(crate) fn take_flow(
        &mut self,
        sid: &str,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<Box<dyn Flow>, String> {
        // 同上：运行态闸门是第一道，先于"取出来"。
        self.dispatch_gate(sid)?;
        if self.is_running(sid) {
            return Err(Self::running_refusal(sid));
        }
        if !self.holds(sid) {
            self.ensure_session(sid, registry, probe)?;
        }
        match self.take(sid) {
            Some(Session::Flow(m)) => {
                self.mark_running(sid.to_string());
                Ok(m)
            }
            Some(other) => {
                self.put(sid.to_string(), other);
                Err("该会话不是流程会话".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：一条单 agent 会话此刻的**身份块**（按当前提示词册与形态现渲染）。
    /// 返回：不是单 agent 会话、或不在世时为 None。
    #[cfg(test)]
    pub fn identity_block(&self, sid: &str) -> Option<String> {
        match self.session(sid) {
            Some(Session::Single(s)) => Some(s.params().identity(&*self.prompt, s.tool_mode())),
            _ => None,
        }
    }

    /// 目的：一个会话的声明：落盘 meta 是唯一真相；取不到（无此会话）就是 None。
    pub(crate) fn session_spec(
        &self,
        sid: &str,
    ) -> Option<crate::units::conversation::session::api::SessionSpec> {
        self.history.load(sid).ok().map(|(m, _)| m.spec)
    }

    /// 目的：这些会话里是否有任何一个的回合还攥在生成线程手里（核心里"生成中"）。
    pub(crate) fn running_any(&self, ids: &[String]) -> bool {
        ids.iter().any(|s| self.is_running(s))
    }

    /// 目的：这一段工作已被停 / 已关闭时，把它的裁决队**整队作废**（等待方按「停止 = 拒绝」解开）。
    /// 返回：要外送 / 落盘的作废事件（本来就没挂着卡 = 空）。判据是落盘的运行态（meta.run），不是内存猜测。
    pub(crate) fn void_gates_of(&mut self, sid: &str) -> Vec<SessionEvent> {
        let reason = match self.history.meta(sid).map(|m| m.run) {
            Ok(RunState::Stopped) => "用户按了停止：这一队裁决一律作废（停止 = 拒绝）",
            Ok(RunState::Archived) => "会话已归档：这一队裁决一律作废",
            _ => return Vec::new(),
        };
        // 判据是**队本身**（核心各关卡与工具级确认排在同一条队上）；没有队 = 本来就没挂着卡。
        let Some(door) = self.desk().peek_gate(sid) else {
            return Vec::new();
        };
        let cards = door.void();
        if cards.is_empty() {
            return Vec::new();
        }
        vec![SessionEvent::DecisionVoid {
            cards,
            reason: reason.to_string(),
        }]
    }

    /// 目的：取这个会话的**裁决队**（核心各关卡与工具级确认共用同一条）。
    /// 约束：新队按转录里已发出的卡号续号——卡号会话内唯一、跨重启稳定（已答过的不重问）。
    pub(crate) fn desk_of(&self, sid: &str) -> Arc<dyn Gates> {
        if let Some(d) = self.desk().peek_gate(sid) {
            return d;
        }
        let issued = self
            .history
            .load(sid)
            .map(|(_, ev)| crate::units::conversation::session::api::issued_in(&ev))
            .unwrap_or(0);
        self.desk_for(sid, issued)
    }

    /// 目的：把 root 及其整棵子树里等用户裁决的队**一律作废**（停止 = 拒绝），按会话给出作废事件。
    /// 约束：会话此刻被生成线程拿在手里时它不在表里——那一份由 put_collab 交回时按运行态作废。
    pub(crate) fn void_gates_in_subtree(&mut self, root: &str) -> Vec<(String, Vec<SessionEvent>)> {
        let mut out = Vec::new();
        for sid in subtree_of(self.history.as_ref(), root) {
            let evs = self.void_gates_of(&sid);
            if !evs.is_empty() {
                out.push((sid, evs));
            }
        }
        out
    }

    /// 目的：协作生成结束**交回**：重新插入 + 解除"生成中"，并**为就绪节点派发子会话**。
    ///  转录已由工作线程按"一次模型调用"的粒度增量落盘（见 `Persister`），这里不重复落。
    ///  返回值：派发产生的事件（调用方负责入台）。
    pub(crate) fn put_collab(
        &mut self,
        sid: &str,
        c: Box<dyn Flow>,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> (Vec<SessionEvent>, Vec<NodeSpawn>) {
        self.unmark_running(sid);
        self.put(sid.to_string(), Session::Flow(c));
        // 交回时这段工作已被停 / 已关闭：队列里没答的卡**一律作废**（生成期间对象在别人手里，只能在这里补）。
        let mut voided = self.void_gates_of(sid);
        let (spawned, todo) = self.spawn_ready_nodes(sid, registry, probe);
        voided.extend(spawned);
        (voided, todo)
    }

    /// 目的：算一个节点子会话的产出摘要，供父会话做总验收。
    /// 参数：`child` = 节点子会话名。
    /// 返回：它**用 submit_report 工具交的回报**；没交回报退回最后一条转录行，仍取不到则为「（该节点没有产出）」。
    /// 约束：会话表只有核心能碰（泵不建会话），所以取产出的判据留在这里。
    pub(super) fn node_note(&self, child: &str) -> String {
        let evs = match self.history.load(child) {
            Ok((_, evs)) => evs,
            Err(_) => Vec::new(),
        };
        let mut lines: Vec<String> = Vec::new();
        let mut reported: Option<String> = None;
        for ev in &evs {
            let Some(ls) = ev.get("lines").and_then(|l| l.as_array()) else {
                continue;
            };
            for l in ls {
                if let Some(t) = l.get("line").and_then(|x| x.as_str()) {
                    lines.push(t.to_string());
                }
                let Some(tool) = l.get("tool") else { continue };
                let name = tool.get("name").and_then(|x| x.as_str()).unwrap_or("");
                if name != crate::units::tools::api::REPORT {
                    continue;
                }
                let out = tool
                    .get("output")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !out.trim().is_empty() {
                    reported = Some(out);
                }
            }
        }
        reported
            .or_else(|| lines.last().cloned())
            .unwrap_or_else(|| "（该节点没有产出）".to_string())
    }

    /// 目的：标记节点完成并记下它的产出，供父会话做总验收。
    /// 参数：`sid` = 协作会话；`node` = 节点名；`note` = 该节点的产出摘要。
    pub(super) fn mark_node_done(&mut self, sid: &str, node: &str, note: &str) {
        if let Some(p) = self
            .session_mut(sid)
            .and_then(|s| s.flow_mut())
            .and_then(|f| f.plan_mut())
        {
            p.finish(node, note);
        }
    }

    /// 目的：给压缩回合准备身份块；切分与模型调用都在工作线程上（界面不被阻塞）。
    /// 返回：本回合的身份块。
    /// 错误：会话不在世或不是单 agent 时如实报错。
    pub fn compact_plan(&mut self, sid: &str, registry: &dyn Registry) -> Result<String, String> {
        // 会话不在世或不是单 agent：**报错**，不给出"空身份块"这种假计划
        // （压缩只对单 agent 会话成立；协作会话由各成员会话按阈值自己压）。
        let model = match self.session(sid) {
            Some(Session::Single(s)) => s.model_id().map(str::to_string),
            _ => {
                return Err(format!(
                    "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                    sid
                ));
            }
        };
        // 手动压缩也要读**当前**设置：用户刚把保留比例调低，不该等会话重建才生效。
        let (window, percent, keep) = self.context_budget(registry, model.as_deref());
        // 身份块要读提示词册：先取一份句柄，免得与会话的可变借用撞车。
        let prompt = Arc::clone(&self.prompt);
        let Some(Session::Single(s)) = self.session_mut(sid) else {
            return Err(format!(
                "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                sid
            ));
        };
        s.set_context_budget(window, percent, keep);
        Ok(s.params().identity(&*prompt, s.tool_mode()))
    }

    /// 目的：本会话的流程自己声明的默认作答选项（核心代答它建的子会话时用；没有就 None）。
    pub(crate) fn default_answer(&self, sid: &str) -> Option<String> {
        self.session(sid)
            .and_then(|s| s.flow())
            .and_then(|f| f.default_answer())
    }

    /// 目的：往某个会话注入一条**系统消息**（讨论的提醒走这条；见 session-model.md 二"系统消息"）。
    ///  返回要外送/落盘的事件。
    pub fn note_system(&mut self, sid: &str, text: &str) -> Result<Vec<SessionEvent>, String> {
        let mut s = self.take_single(sid)?;
        let events = s.note_system(text);
        self.put_single(sid, s);
        self.persister(sid).persist(&events);
        Ok(events)
    }

    /// 目的：给某个会话接下来的行打上**整场工作的下一个回合 id**（节点执行也用同一套编号）。
    ///  一个 agent 一个会话：它的回合计数来自父会话——回档同步靠两边同一套编号。
    pub(crate) fn bump_turn_of_child(&mut self, child: &str) -> u64 {
        let parent = self
            .history
            .load(child)
            .ok()
            .and_then(|(m, _)| m.parent.clone());
        let Some(parent) = parent else {
            return 0;
        };
        let tid = self
            .session_mut(&parent)
            .and_then(|s| s.flow_mut())
            .map(|f| f.turn_cursor())
            .unwrap_or(0);
        if let Some(Session::Single(s)) = self.session_mut(child) {
            s.set_turn(tid);
        }
        tid
    }

    /// 目的：把当前设置与模型窗口刷到一条活会话（设置改了不用重建会话）。
    /// 参数：`s` = 单 agent 会话；窗口按它自己的模型解析。
    pub(crate) fn refresh_context_budget(&self, registry: &dyn Registry, s: &mut AgentSession) {
        let model = s.model_id().map(str::to_string);
        let (window, percent, keep) = self.context_budget(registry, model.as_deref());
        s.set_context_budget(window, percent, keep);
    }
}
