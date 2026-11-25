//! 目的：回合状态——会话在核心线程里的在世表、运行态与创建占位、裁决队、每会话的 shell 活进程槽。
//! 管：`Session` 两种在世形态（单 agent / 一个流程槽）的在世表；运行态与创建占位的标记；每会话的裁决通道；
//!   每会话的 shell 活进程槽（同一会话的多次 shell 调用共用一份活进程）。
//! 不管：会话本体（`session` / `collab`）、生成驱动、落盘；谁在什么时候放取会话由 `runtime` 的用例决定。
//! 联动：`runtime` 经 `api` 读写；裁决通道来自 `session::api`，shell 槽是 `session::api::ShellSlot`。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use crate::kernel::api::types::SessionId;
use crate::units::agentloop::api::{AgentSession, ShellSlot};
use crate::units::conversation::session::api::{Decisions, Flow, Gates, decisions};

/// 目的：一份在世会话的两种形态：单 agent（含代理会话与子会话）与一个**流程**（协作只是已注册的一个流程）。
/// 约束：流程槽是装箱的 trait 对象——会话形态不再是一张封闭清单，新流程不改这里。
///   两变体大小仍差得远（见 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::large_enum_variant)]
pub enum Session {
    /// 目的：单 agent 会话本体。
    Single(AgentSession),
    /// 目的：流程槽（协作、以后的评审流程都装在这里）。
    Flow(Box<dyn Flow>),
}

impl Session {
    /// 目的：流程自己判定整场工作已收尾（单 agent 会话永远没有这个状态）。
    pub(crate) fn is_done(&self) -> bool {
        match self {
            Session::Flow(m) => m.is_done(),
            Session::Single(_) => false,
        }
    }

    /// 目的：流程面（只读）：驱动处置裁决票、问「这一关要不要就地收尾」都走它。
    pub(crate) fn flow(&self) -> Option<&dyn Flow> {
        match self {
            Session::Flow(m) => Some(&**m),
            Session::Single(_) => None,
        }
    }

    /// 目的：流程面（可写一份）。
    pub(crate) fn flow_mut(&mut self) -> Option<&mut dyn Flow> {
        match self {
            Session::Flow(m) => Some(&mut **m),
            Session::Single(_) => None,
        }
    }
}

/// 目的：核心线程持有的回合状态；一个进程一份，只在核心线程上读写。
pub struct TurnState {
    /// 目的：在世会话表（键是会话名）。
    sessions: HashMap<SessionId, Session>,
    /// 目的：正在生成的会话（生成期间会话本体被取出，表里不留对象）。
    running: BTreeSet<SessionId>,
    /// 目的：正在创建 / 启动的会话（创建占位，防止并发建同名会话）。
    starting: BTreeSet<String>,
    /// 目的：全会话的裁决通道表（每会话一条，同会话一次只问一条）。
    desk: Arc<dyn Decisions>,
    /// 目的：每会话的 shell 活进程槽（同一会话的 shell 调用共用一个活进程）。
    shell_slots: Mutex<BTreeMap<String, Arc<ShellSlot>>>,
}

impl Default for TurnState {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnState {
    /// 目的：造一份空的回合状态（裁决通道表随之就绪）。
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            running: BTreeSet::new(),
            starting: BTreeSet::new(),
            desk: decisions(),
            shell_slots: Mutex::new(BTreeMap::new()),
        }
    }

    /// 目的：取一份在世会话的只读引用。
    pub fn session(&self, sid: &str) -> Option<&Session> {
        self.sessions.get(sid)
    }

    /// 目的：取一份在世会话的可写引用。
    pub fn session_mut(&mut self, sid: &str) -> Option<&mut Session> {
        self.sessions.get_mut(sid)
    }

    /// 目的：这份会话是否在世。
    pub fn holds(&self, sid: &str) -> bool {
        self.sessions.contains_key(sid)
    }

    /// 目的：把会话放进在世表（生成结束后放回，或新建时放入）。
    pub fn put(&mut self, sid: impl Into<SessionId>, session: Session) {
        self.sessions.insert(sid.into(), session);
    }

    /// 目的：从在世表取出会话（生成开始时取出，表里不留对象）。
    pub fn take(&mut self, sid: &str) -> Option<Session> {
        self.sessions.remove(sid)
    }

    /// 目的：这份会话是否正在生成。
    pub fn is_running(&self, sid: impl AsRef<str>) -> bool {
        self.running.contains(sid.as_ref())
    }

    /// 目的：标记开始生成（生成期间拒绝改配 / 回档 / 删除）。
    pub fn mark_running(&mut self, sid: impl Into<SessionId>) {
        self.running.insert(sid.into());
    }

    /// 目的：撤销运行态（区间结束或中断收尾）。
    pub fn unmark_running(&mut self, sid: impl AsRef<str>) {
        self.running.remove(sid.as_ref());
    }

    /// 目的：正在生成的会话（工作列表要如实列出，虽然对象在工作线程上）。
    pub fn running_iter(&self) -> impl Iterator<Item = &SessionId> {
        self.running.iter()
    }

    /// 目的：遍历在世会话表（只读，工作列表与界面用）。
    pub fn sessions_iter(&self) -> impl Iterator<Item = (&SessionId, &Session)> {
        self.sessions.iter()
    }

    /// 目的：这份会话是否正在创建 / 启动。
    pub fn is_starting(&self, sid: impl AsRef<str>) -> bool {
        self.starting.contains(sid.as_ref())
    }

    /// 目的：占住创建位（并发建同名会话时后到的那次直接拒绝）。
    pub fn mark_starting(&mut self, sid: impl Into<String>) {
        self.starting.insert(sid.into());
    }

    /// 目的：放开创建位。
    pub fn unmark_starting(&mut self, sid: impl AsRef<str>) {
        self.starting.remove(sid.as_ref());
    }

    /// 目的：全会话的裁决通道表。
    pub fn desk(&self) -> &Arc<dyn Decisions> {
        &self.desk
    }

    /// 目的：一份会话的裁决闸：已开队直接取，未开队按 `issued` 现开一条。
    /// 参数：`issued` = 会话落盘里的裁决代号；同一条队重复取到的是同一把闸。
    pub fn desk_for(&self, sid: &str, issued: u64) -> Arc<dyn Gates> {
        self.desk.gate(sid, issued)
    }

    /// 目的：读会话最近一次 shell 的快照；没跑过就是 None（状态栏据此决定要不要出 chip）。
    pub fn shell_view(&self, sid: &str) -> Option<crate::units::agentloop::api::ShellLive> {
        self.shell_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .and_then(|s| s.get())
    }

    /// 目的：这份会话的 shell 活进程槽（第一次用时建，之后共用）。
    pub fn shell_slot(&self, sid: &str) -> Arc<ShellSlot> {
        let mut map = self.shell_slots.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            map.entry(sid.to_string())
                .or_insert_with(|| Arc::new(ShellSlot::default())),
        )
    }
}
