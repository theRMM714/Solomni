//! 目的：会话的**在世表协议**——取出 / 交回 / 运行态闸门 / 裁决队作废 / 派生子会话。
//! 管：`take_single` / `take_collab` / `take_collab_raw`（交给工作线程）、`put_single` / `put_single_recorded` /
//!   `put_collab` / `abort_running`（交回与崩溃）、`void_gates_of` / `void_gates_in_subtree` / `desk_of` /
//!   `forget_desk`（裁决队）、`spawn_agent_session` / `spawn_sub_session`（派生）、`node_note` / `mark_node_done`（节点产出）、`notice_row`（内容类提示记行）。
//! 不管：事件怎么落盘（交回时把要落的事件交给调用方）、协作链怎么推进（`conductor` 的 `flow`）、生成驱动。
//! 联动：`conductor` 的生成驱动与流水线经 `turn::api` 取用；装配与重建见 `assemble.rs` / `rebuild.rs`。

use std::sync::Arc;

use crate::capabilities::collab::api::CollabSession;
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{
    AgentSession, DecisionDoor, RunState, SessionEvent, SessionMeta,
};
use crate::capabilities::turn::api::{HandlerSource, Session};
use crate::kernel::ports::HostProbe;

use crate::capabilities::turn::service::session::Turn;
use crate::kernel::api::now_ts;

/// 目的：一笔交回的**落点**：叫醒谁、替谁落盘。
/// 约束：`turn` 不落盘——要写进代理父会话的那条通知行原样交回，由调用方在同一处落。
pub struct PutBack {
    /// 目的：这笔交回所属的父会话（`None` = 没有父会话，调用方不叫醒任何人）。
    pub parent: Option<String>,
    /// 目的：要写进代理父会话的那条通知行（会话名 + 事件）。
    pub notice: Option<(String, Vec<SessionEvent>)>,
}

impl Turn {
    /// 目的：把单 agent 会话**交给工作线程**（核心表里留"生成中"）。
    ///  取出的窗口内，核心队列是空的——读接口与其它会话的命令因此照常。
    pub(crate) fn take_single(&mut self, sid: &str) -> Result<AgentSession, String> {
        // 运行态闸门：暂停 / 关闭的会话不启动任何生成（唤醒与派发都从这里过）。
        self.dispatch_gate(sid)?;
        if self.is_running(sid) {
            return Err(Turn::running_refusal(sid));
        }
        match self.take_session(sid) {
            Some(Session::Single(s)) => {
                self.mark_running(sid);
                Ok(s)
            }
            Some(other) => {
                // 不是单 agent：原样放回，交给协作那条路（B-1 只搬单 agent）。
                self.put(sid, other);
                Err("该会话不是单 agent 模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：把协作会话**交给工作线程**（核心表里留"生成中"）。
    ///  会话还没装进内存时先从落盘重建：协作的"继续"可能先于"打开"到达。
    /// 参数：`sid` = 会话 id；`registry` = 登记处；`probe` = 宿主能力探测；`handlers` = 核心自有工具处理器注入。
    pub(crate) fn take_collab(
        &mut self,
        sid: &str,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        handlers: &HandlerSource<'_>,
    ) -> Result<CollabSession, String> {
        // 同上：运行态闸门是第一道，先于"取出来"。
        self.dispatch_gate(sid)?;
        if self.is_running(sid) {
            return Err(Turn::running_refusal(sid));
        }
        if !self.holds(sid) {
            self.ensure_session(sid, registry, probe, handlers)?;
        }
        match self.take_session(sid) {
            Some(Session::Collab(c)) => {
                self.mark_running(sid);
                Ok(c)
            }
            Some(other) => {
                self.put(sid, other);
                Err("该会话不是协作模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：裸搬协作会话（不触发任务链派发那套副作用）：驱动循环每步都要搬一次。
    pub(crate) fn take_collab_raw(&mut self, sid: &str) -> Result<CollabSession, String> {
        match self.take_session(sid) {
            Some(Session::Collab(c)) => Ok(c),
            Some(other) => {
                self.put(sid, other);
                Err("该会话不是协作模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：这一段工作已被停 / 已关闭时，把它的裁决队**整队作废**（等待方按「停止 = 拒绝」解开）。
    /// 返回：要外送 / 落盘的作废事件（本来就没挂着卡 = 空）。判据是落盘的运行态（meta.run），不是内存猜测。
    pub(crate) fn void_gates_of(&mut self, sid: &str) -> Vec<SessionEvent> {
        let reason = match self.history.meta(sid).map(|m| m.run) {
            Ok(RunState::Stopped) => "用户按了停止：这一队裁决一律作废（停止 = 拒绝）",
            Ok(RunState::Closed) => "会话已关闭：这一队裁决一律作废",
            _ => return Vec::new(),
        };
        // 判据是**队本身**（核心各关卡与工具级确认排在同一条队上）；没有队 = 本来就没挂着卡。
        let Some(door) = self.desk().peek(sid) else {
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
    pub(crate) fn desk_of(&self, sid: &str) -> Arc<DecisionDoor> {
        if let Some(d) = self.desk().peek(sid) {
            return d;
        }
        let issued = self
            .history
            .load(sid)
            .map(|(_, ev)| crate::capabilities::session::api::issued_in(&ev))
            .unwrap_or(0);
        self.desk().session(sid, issued)
    }

    /// 目的：会话没了，它的裁决队随会话一起消失（等待方与转录都不复存在）。
    pub(crate) fn forget_desk(&self, sid: &str) {
        self.desk().forget(sid);
    }

    /// 目的：这些会话里是否有任何一个的回合还攥在生成线程手里（核心里"生成中"）。
    pub(crate) fn running_any(&self, ids: &[String]) -> bool {
        self.any_running(ids)
    }

    /// 目的：把 root 及其整棵子树里等用户裁决的队**一律作废**（停止 = 拒绝），按会话给出作废事件。
    /// 约束：会话此刻被生成线程拿在手里时它不在表里——那一份由 put_collab 交回时按运行态作废。
    pub(crate) fn void_gates_in_subtree(&mut self, root: &str) -> Vec<(String, Vec<SessionEvent>)> {
        let mut out = Vec::new();
        for sid in self.subtree_of(root) {
            let evs = self.void_gates_of(&sid);
            if !evs.is_empty() {
                out.push((sid, evs));
            }
        }
        out
    }

    /// 目的：协作生成结束**交回**：重新插入 + 解除"生成中"，并把这些工作已被停 / 已关闭时
    ///  队列里没答的卡**一律作废**（生成期间对象在别人手里，只能在这里补）。
    ///  转录已由工作线程按"一次模型调用"的粒度增量落盘（见 `Persister`），这里不重复落。
    /// 返回：作废事件（调用方负责外送与入台，并按需派发就绪节点）。
    pub(crate) fn put_collab(&mut self, sid: &str, c: CollabSession) -> Vec<SessionEvent> {
        self.unmark_running(sid);
        self.put(sid, Session::Collab(c));
        self.void_gates_of(sid)
    }

    /// 目的：一个子会话的**产出**（节点交付物）：它用 submit_report 工具交的回报优先，
    ///  没有就退回"最后一条转录行"（如实取到的东西），仍取不到就说"没有产出"。
    pub(crate) fn node_note(&self, child: &str) -> String {
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
                if name != crate::capabilities::tools::api::REPORT {
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

    /// 目的：标记节点完成（记下它的产出），供父会话做总验收。
    /// 参数：`sid` = 父会话；`node` = 节点名；`note` = 该节点的产出摘要。
    /// 返回：无；会话不在世或不是协作形态时什么都不做。
    pub(crate) fn mark_node_done(&mut self, sid: &str, node: &str, note: &str) {
        if let Some(Session::Collab(c)) = self.session_mut(sid) {
            c.mark_node_done(node, note);
        }
    }

    /// 目的：为一个 agent 建它的会话（名单确认时建）：节点执行与讨论回合**共用同一个**会话。
    ///  复用"按 meta 重建"的整条装配路径——子会话与用户建的会话**没有第二种实现**。
    /// 参数：`parent` = 父会话名；`agent` = 名单里的 agent 名；`registry` / `probe` / `handlers` 同 `take_collab`。
    pub(crate) fn spawn_agent_session(
        &mut self,
        parent: &str,
        agent: &str,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        handlers: &HandlerSource<'_>,
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
        self.ensure_session(&child, registry, probe, handlers)?;
        Ok(child)
    }

    /// 目的：为任务链的一个节点建**子会话**：它就是该节点负责人的 agent 会话（一个 agent 一个会话）。
    pub(crate) fn spawn_sub_session(
        &mut self,
        parent: &str,
        node: &str,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        handlers: &HandlerSource<'_>,
    ) -> Result<String, String> {
        let (_pmeta, _) = self.history.load(parent)?;
        let assignee = {
            let c = match self.session(parent) {
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
        self.spawn_agent_session(parent, &assignee, registry, probe, handlers)
    }

    /// 目的：代理模式：一个子会话停下了 → 往**代理会话**写一条通知行（只写这一条，不转发子会话转录）。
    /// 返回：被通知的代理会话名 + 要落进它的事件；父不是代理会话（协作节点等）就返回 None、什么都不做。
    /// 约束：turn 不落盘——事件原样交给调用方，由它在同一处落。
    pub(crate) fn notify_proxy_of_child(
        &mut self,
        child: &str,
    ) -> Option<(String, Vec<SessionEvent>)> {
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
        let evs = ps.note_task(&format!(
            "[子会话] {} 这一轮结束{}。要看它说了什么用 read_session_messages（0 = 最新）；要它继续或返工用 send_session_message。",
            child, gate
        ));
        self.put_single(&parent, ps);
        Some((parent, evs))
    }

    /// 目的：生成结束**交回**：重新插入 + 解除"生成中"，并标记正跑在这个会话里的节点已交付。
    ///  转录**已由工作线程按"一轮一次"的粒度增量落盘**（见 `Persister`），这里不重复落。
    /// 返回：叫醒谁（父会话）与要替代理父会话落盘的通知行——顺序与原地实现一致：节点标记在前，代理通知在后。
    pub(crate) fn put_single(&mut self, sid: &str, s: AgentSession) -> PutBack {
        self.unmark_running(sid);
        self.put(sid, Session::Single(s));
        // 子会话完成 = 它的节点交付了：标记**正跑在这个会话里的那个节点**（产出即交付物），并交回父会话。
        // 一个 agent 一个会话（可能依次服务多个节点），所以按 sub_session 认节点，不靠 meta.node。
        let Some((meta, _)) = self.history.load(sid).ok() else {
            return PutBack {
                parent: None,
                notice: None,
            };
        };
        let Some(parent) = meta.parent.clone() else {
            return PutBack {
                parent: None,
                notice: None,
            };
        };
        let note = self.node_note(sid);
        let node = match self.session_mut(&parent) {
            Some(Session::Collab(c)) => c.running_node_of(sid),
            _ => None,
        };
        if let Some(node) = node {
            self.mark_node_done(&parent, &node, &note);
        }
        // 代理模式的父：把"这个子会话停下了"如实告诉它（**不转发子会话转录**），
        // 由调用方按形态叫醒父会话。
        let notice = self.notify_proxy_of_child(sid);
        PutBack {
            parent: Some(parent),
            notice,
        }
    }

    /// 目的：同步路径的交回：重新插入 + 解除"生成中"（整段落盘由调用方紧随其后做）。
    pub(crate) fn put_single_recorded(&mut self, sid: &str, s: AgentSession) {
        self.unmark_running(sid);
        self.put(sid, Session::Single(s));
    }

    /// 目的：生成线程崩溃：会话对象随线程一起没了，但**转录在盘上**。
    ///  只解除"生成中"，下次访问按落盘转录重建——绝不把会话卡在"生成中"。
    pub(crate) fn abort_running(&mut self, sid: &str) {
        self.unmark_running(sid);
    }

    /// 目的：把一条内容类系统提示记成**转录行**（有 id、可回档/折叠）；返回要外送的事件。
    /// 约束：只在会话对象在核心表里时分配行 id；生成中的会话被取走、没有计数器可分配，
    ///   这时如实退回瞬时提示（界面照样看得见，只是不可回档）。
    pub(crate) fn notice_row(&mut self, sid: &str, text: &str) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        match self.session_mut(sid) {
            Some(Session::Single(s)) => out.extend(s.note_notice(text)),
            Some(Session::Collab(c)) => c.notice(text, &mut |e| out.push(e)),
            None => out.push(SessionEvent::Notice(text.to_string())),
        }
        out
    }
}
