//! 目的：会话在世表与运行态的状态容器——`TurnState` 与 `Session` 枚举的实现处。
//! 管：五块状态的持有与全部访问器：会话对象存取与遍历、运行态标记、创建占名、裁决队句柄、shell 槽。
//! 不管：会话怎么建、回合怎么驱动、事件怎么落盘（仍在 `conductor` 的 service）；不碰任何 IO。
//! 联动：`conductor` 的 service 各文件经本能力的 `api` 取 `TurnState` 与 `Session`；单元地图见 docs/turn/unit-map.md。

use crate::capabilities::collab::api::CollabSession;
use crate::capabilities::session::api::{AgentSession, DecisionDesk, ShellLive, ShellSlot};
use crate::kernel::api::SessionId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

/// 目的：会话实例：单 agent 会话或协作会话（本体自带端口，可跨线程移动）。
///  两变体大小差得远（协作会话带整份讨论状态），装箱只换来一次间接寻址、
///  却把"会话本体可直接移动"这个形状改掉——有意不装箱（见 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::large_enum_variant)]
pub enum Session {
    Single(AgentSession),
    Collab(CollabSession),
}

/// 目的：会话在世表与运行态的唯一持有者（核心线程独占，不加锁；只有 shell 槽自带一把锁）。
pub struct TurnState {
    /// 目的：会话在世表：键是会话 id；对象交到工作线程手里期间这里没有它。
    sessions: HashMap<SessionId, Session>,
    /// 目的：正在生成的会话：对象被工作线程**取走**了，核心表里暂时没有它。
    ///  为什么取出而不是就地生成：生成要跑几十秒到几分钟，占着唯一的命令队列会让
    ///  读接口（历史列表、状态）与其它会话的命令全排在它后面——界面因此"假死"。
    ///  这一态只表示"不在表里是因为在生成"，不是"不存在"。
    running: BTreeSet<SessionId>,
    /// 目的：正在准备中的会话名（创建分两段：核心先占名与预备，慢的那一段在调用方线程上跑）。
    ///  有了它，同名并发创建就不会都通过"名字不存在"那道检查而互相覆盖。
    starting: BTreeSet<String>,
    /// 目的：**裁决队**（按会话 id 取）：核心各关卡与工具级确认共用同一条队（见 docs/session/session-model.md）。
    ///  它跨线程：生成线程等在工作线程上，回答与快照在核心线程上——同一份句柄、同一把锁。
    desk: Arc<DecisionDesk>,
    /// 目的：会话 → shell 实时状态槽（状态栏卡片用）；工具循环写、SessionView 读。
    shell_slots: Mutex<BTreeMap<String, Arc<ShellSlot>>>,
}

/// 目的：五块状态一律空着起步（裁决队是一份新建的空队）。
impl Default for TurnState {
    fn default() -> TurnState {
        TurnState {
            sessions: HashMap::new(),
            running: BTreeSet::new(),
            starting: BTreeSet::new(),
            desk: DecisionDesk::new(),
            shell_slots: Mutex::new(BTreeMap::new()),
        }
    }
}

impl TurnState {
    /// 目的：五块状态一律空着起步（等价于 `TurnState::default()`）。
    pub fn new() -> TurnState {
        TurnState::default()
    }

    /// 目的：取这个会话的实例。
    /// 参数：`sid` = 会话 id。
    /// 返回：不在世（含对象已交给工作线程）时为 `None`。
    pub fn session(&self, sid: &str) -> Option<&Session> {
        self.sessions.get(sid)
    }

    /// 目的：取这个会话的可变实例（核心线程独占，改完即生效）。
    /// 参数：`sid` = 会话 id。
    /// 返回：不在世（含对象已交给工作线程）时为 `None`。
    pub fn session_mut(&mut self, sid: &str) -> Option<&mut Session> {
        self.sessions.get_mut(sid)
    }

    /// 目的：这个会话是否在世。
    /// 参数：`sid` = 会话 id。
    /// 约束：对象交给工作线程期间它不在表里，这里如实返回 false（见 `is_running`）。
    pub fn holds(&self, sid: &str) -> bool {
        self.sessions.contains_key(sid)
    }

    /// 目的：把会话放进在世表；同 id 已在世时覆盖。
    /// 参数：`sid` = 会话 id；`s` = 会话实例。
    pub fn put(&mut self, sid: &str, s: Session) {
        self.sessions.insert(sid.to_string(), s);
    }

    /// 目的：把会话从在世表取出（交出给工作线程走这条）。
    /// 参数：`sid` = 会话 id。
    /// 返回：取出来的会话；不在世时为 `None`。
    pub fn take(&mut self, sid: &str) -> Option<Session> {
        self.sessions.remove(sid)
    }

    /// 目的：遍历在世的会话（读接口与派发按这张表取事实）。
    /// 返回：`(会话 id, 会话)` 的迭代器，顺序不承诺。
    pub fn iter(&self) -> impl Iterator<Item = (&SessionId, &Session)> {
        self.sessions.iter()
    }

    /// 目的：这个会话的回合是否还攥在生成线程手里。
    /// 参数：`sid` = 会话 id。
    pub fn is_running(&self, sid: &str) -> bool {
        self.running.contains(sid)
    }

    /// 目的：标记这个会话的回合已交给生成线程（此后它不在在世表里）。
    /// 参数：`sid` = 会话 id。
    pub fn mark_running(&mut self, sid: &str) {
        self.running.insert(sid.to_string());
    }

    /// 目的：解除这个会话的运行态（回合交回或中止时调）。
    /// 参数：`sid` = 会话 id。
    pub fn unmark_running(&mut self, sid: &str) {
        self.running.remove(sid);
    }

    /// 目的：遍历正在生成的会话 id（状态栏与停止指令按它取事实）。
    /// 返回：会话 id 的迭代器，顺序不承诺。
    pub fn running_iter(&self) -> impl Iterator<Item = &SessionId> {
        self.running.iter()
    }

    /// 目的：这些会话里是否有任何一个的回合还攥在生成线程手里。
    /// 参数：`ids` = 会话 id 清单（空清单恒为 false）。
    pub fn any_running(&self, ids: &[String]) -> bool {
        ids.iter().any(|s| self.running.contains(s))
    }

    /// 目的：这个会话名是否已被一次创建占住（创建分两段，慢的那一段在调用方线程上跑）。
    /// 参数：`name` = 会话名。
    pub fn is_starting(&self, name: &str) -> bool {
        self.starting.contains(name)
    }

    /// 目的：占住一个会话名：同名并发创建因此不会都通过「名字不存在」那道检查而互相覆盖。
    /// 参数：`name` = 会话名。
    pub fn mark_starting(&mut self, name: &str) {
        self.starting.insert(name.to_string());
    }

    /// 目的：放掉创建占名（创建成功或失败时都要调，别让名字一直占着）。
    /// 参数：`name` = 会话名。
    pub fn unmark_starting(&mut self, name: &str) {
        self.starting.remove(name);
    }

    /// 目的：裁决队句柄：核心各关卡与工具级确认共用同一条队。
    /// 返回：跨线程共享的队（同一份句柄、同一把锁）。
    pub fn desk(&self) -> &DecisionDesk {
        &self.desk
    }

    /// 目的：取（必要时创建）一个会话的 shell 实时状态槽；成员工具面与状态栏共用同一份。
    /// 参数：`sid` = 会话 id。
    pub fn shell_slot(&self, sid: &str) -> Arc<ShellSlot> {
        let mut map = self.shell_slots.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            map.entry(sid.to_string())
                .or_insert_with(|| Arc::new(ShellSlot::default())),
        )
    }

    /// 目的：读会话最近一次 shell 的快照。
    /// 参数：`sid` = 会话 id。
    /// 返回：没运行过就是 `None`（状态栏据此决定要不要出 chip）。
    pub fn shell_view(&self, sid: &str) -> Option<ShellLive> {
        self.shell_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .and_then(|s| s.get())
    }
}
