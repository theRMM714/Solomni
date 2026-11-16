//! 目的：turn 能力的入站契约：回合状态的类型与访问器、事件台、生成入口，以及宿主必须提供的入口（`TurnHost`）。
//! 管：重导出 `service::state` 的 `Session`、`service::session` 的 `Turn`、装配入口与参数对象、
//!   事件台与生成入口（`service::generation`），并定义宿主端口 `TurnHost`。
//! 不管：状态实现（`service/state.rs`）、推进规则（`service/{turn,flow}`）、谁在什么时候放取会话（宿主）。
//! 联动：宿主（命令队列手柄）实现 `TurnHost`；呈现层与测试只经本模块取用。

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// 目的：一席工具面与单 agent 会话对象的装配入口（宿主取完事实后转调这里）。
pub use crate::capabilities::turn::service::assemble::member_tools;
/// 目的：装配用的三类参数对象：能力面（`SessionDeps`）、一席事实（`MemberSpec`）、单 agent 一场事实（`SingleSpec`）。
#[allow(unused_imports)]
pub use crate::capabilities::turn::service::assemble::{MemberSpec, SessionDeps, SingleSpec};
/// 目的：生成入口（单 agent / 节点 / 协作 / 成员 / 压缩）与脱离调用方的点火器。
pub(crate) use crate::capabilities::turn::service::generation::{
    collab_generation, compact, single_generation, spawn_detached_collab,
    spawn_detached_collab_answer, spawn_detached_continue, spawn_detached_dispose,
    spawn_detached_node, spawn_detached_proxy, start_child_collab,
};
/// 目的：一次模型调用的通道参数（测试访问器与生产共用这一份判据）。
#[cfg(test)]
pub use crate::capabilities::turn::service::model::llm_opts;
/// 目的：回放一段转录得到对话与压缩边界（重建会话与回档都只经这里）。
#[allow(unused_imports)]
pub use crate::capabilities::turn::service::rebuild::replay_dialogue;
/// 目的：代理会话的私有工作目录名与核心说话人名称（宿主建代理会话时要这两样）。
pub use crate::capabilities::turn::service::rebuild::{PROXY_PRIVATE_DIR, SPEAKER};
/// 目的：回合状态与会话本体的类型出口。
pub use crate::capabilities::turn::service::session::Turn;

/// 目的：造这个会话的**工具层提问端口**（登记一张卡、外送并落盘、停会话都走同一会话那条道）。
/// 参数：`door` = 该会话的裁决通道；`sid` = 会话名；`emit` = 推一条事件；`stop` = 停这个会话。
/// 返回：可跨线程用的提问端口。
pub fn session_ask(
    door: Arc<dyn Gates>,
    sid: &str,
    emit: Box<dyn Fn(SessionEvent) + Send + Sync>,
    stop: Box<dyn Fn(&str) + Send + Sync>,
) -> Arc<dyn AskUser> {
    Arc::new(crate::capabilities::turn::service::ask_user::SessionAsk::new(door, sid, emit, stop))
}
/// 目的：在世会话形态（宿主与测试只经它取用）。
///   回合状态本身（`TurnState`）是本能力的内部结构：它只被 `Turn` 持有，不出单元。
pub use crate::capabilities::turn::service::state::Session;

use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{
    AgentSession, DecisionQueue, Gates, Persister, SessionEvent, SessionMeta,
};
use crate::kernel::api::SessionId;
use crate::kernel::ports::{AskUser, HostProbe};

/// 目的：一次生成的输出方式：流式逐片，还是只要最终结果。
///   「怎么呈现」是呈现层的决定，所以由调用方显式给出，核心不替它猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Stream,
    Final,
}

/// 目的：一次会话推进的结果：**只回事件台的头部序号**。
///   命令不携带事实——事实只有一条来路（事件台）：想看就按 `since` 订阅，
///   谁发起的命令都一样（CLI / Web / 桌面 / 演示脚本）。
#[derive(Debug, Clone)]
pub struct Advance {
    pub head: u64,
}

/// 目的：事件台上的一条：全局序号 + 会话 + 事实。
#[derive(Debug, Clone)]
pub struct EventLine {
    pub seq: u64,
    pub sid: SessionId,
    pub events: Vec<SessionEvent>,
}

/// 目的：事件上限与保留量：本地单机足够，防无界增长。
const BUS_MAX: usize = 10_000;
const BUS_KEEP: usize = 5_000;

/// 目的：事件发布台：核心是唯一生产者（`push` 只由本能力调用）；
///   任何数量的消费者按 `since` 增量取——多端并用、断线重连都不用额外机制。
pub struct EventBus {
    inner: Mutex<BusInner>,
}

#[derive(Default)]
struct BusInner {
    seq: u64,
    lines: Vec<EventLine>,
    /// 目的：每个会话最近一次回档的重置点（事件台序号）：该序号及之前的事实已被盘上转录取代，
    /// 历史合流不得再把它当实时尾巴；客户端自己的 since 游标不受影响（它们仍需看到
    /// 重置事件本身来清空视图，见 push_rewound）。
    rewound: BTreeMap<String, u64>,
}

impl EventBus {
    pub fn new() -> Arc<EventBus> {
        Arc::new(EventBus {
            inner: Mutex::new(BusInner::default()),
        })
    }

    /// 目的：入箱一批事实并裁剪，返回它的全局序号（调用方已持锁）。
    fn append_locked(g: &mut BusInner, sid: &str, events: &[SessionEvent]) -> u64 {
        g.seq += 1;
        let seq = g.seq;
        g.lines.push(EventLine {
            seq,
            sid: sid.to_string(),
            events: events.to_vec(),
        });
        if g.lines.len() > BUS_MAX {
            let drop = g.lines.len() - BUS_KEEP;
            g.lines.drain(..drop);
        }
        seq
    }

    /// 目的：入箱一批事实，返回它的全局序号。核心内部调用；锁中毒不算致命（拿回内层继续）。
    pub(crate) fn push(&self, sid: &str, events: &[SessionEvent]) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::append_locked(&mut g, sid, events)
    }

    /// 目的：回档成功后入箱一条 `rewound`（携带活动视图），并把该会话的重置水印推进到它的序号。
    /// 约束：盘上转录已经被回档改写，事件台本身不能回退；没有这条事实，刷新/多端无法知道旧事件已作废。
    pub fn push_rewound(&self, sid: &str, events: Vec<serde_json::Value>) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ev = SessionEvent::Rewound { events };
        let seq = Self::append_locked(&mut g, sid, std::slice::from_ref(&ev));
        g.rewound.insert(sid.to_string(), seq);
        seq
    }

    /// 目的：事件台当前头部序号：命令回包只给这个，调用方拿它当**订阅起点**。
    pub fn head(&self) -> u64 {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.seq
    }

    /// 目的：测试专用：往事件台放一条空事件批，让长轮询立刻返回（不真等 20 秒）。
    #[cfg(test)]
    pub(crate) fn seed_for_test(&self, sid: &str) -> u64 {
        self.push(sid, &[])
    }

    /// 目的：取 `since` 之后的事件批 + 当前头部 + **最老还留着的序号**（同一把锁内）。
    ///   为什么要把 oldest 给客户端：事件台会裁剪（BUS_MAX/BUS_KEEP），`since` 之后那一小段可能
    ///   已经永久没了。客户端据此**重新对齐**（拉一次历史重放），而不是按 seq 干等——干等的结果
    ///   是后续批次全部滞留，只有刷新页面才恢复。
    pub fn snapshot(&self, sid: Option<&str>, since: u64) -> (Vec<EventLine>, u64, u64) {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let lines = g
            .lines
            .iter()
            .filter(|l| l.seq > since && sid.is_none_or(|x| l.sid == x))
            .cloned()
            .collect();
        let oldest = g.lines.first().map(|l| l.seq).unwrap_or(g.seq + 1);
        (lines, g.seq, oldest)
    }

    /// 目的：历史合流窗口：只取该会话**最近一次回档之后**的事件批 + 当前头部（同一把锁内）。
    /// 回档水印之前的批次已被盘上转录取代，不能再拼回历史视图；客户端的长轮询仍用
    /// `snapshot`（它需要看到重置事件本身来清空视图）。
    fn snapshot_after_rewind(&self, sid: &str) -> (Vec<EventLine>, u64) {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let cut = g.rewound.get(sid).copied().unwrap_or(0);
        let lines = g
            .lines
            .iter()
            .filter(|l| l.sid == sid && l.seq > cut)
            .cloned()
            .collect();
        (lines, g.seq)
    }

    /// 目的：盘上转录**之外**的实时尾巴 + 事件台当前头部：给「历史和实时一次给全」用（见呈现层的 history.open）。
    ///   只取「**转录在事件台里的最后一个位置之后**」的事实：转录是事件台的**前缀**，
    ///   它之前/之中的短暂事件（流式增量、开跑那条运行态）已经被盘上的定稿行取代了；
    ///   把它们也塞进尾巴，前端会先画定稿行、再画一个永远填不上的空"谁正在说"块。
    ///   判据是**结构化相等**（两边 JSON 出自同一套序列化器，就是同一个值），不是按行 id 猜：
    ///   `notice`/`node_started` 这类行本来就没有 id，按 id 去重正是刷新后整段重复的来源。
    ///   返回的尾巴按事件台顺序展开，前端因此**只按序 append**，不自己合并两个来源。
    pub fn tail_excluding(
        &self,
        sid: &str,
        transcript: &[serde_json::Value],
    ) -> (Vec<serde_json::Value>, u64) {
        let mut left: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for ev in transcript {
            if let Ok(k) = serde_json::to_string(ev) {
                *left.entry(k).or_insert(0) += 1;
            }
        }
        let (lines, head) = self.snapshot_after_rewind(sid);
        let mut on_bus: Vec<serde_json::Value> = lines
            .into_iter()
            .flat_map(|l| l.events.into_iter().map(|e| e.to_json()))
            .collect();
        let mut start = 0usize;
        for (i, v) in on_bus.iter().enumerate() {
            if let Ok(k) = serde_json::to_string(v)
                && let Some(n) = left.get_mut(&k)
                && *n > 0
            {
                *n -= 1;
                start = i + 1;
            }
        }
        let mut tail = Vec::new();
        for v in on_bus.drain(..).skip(start) {
            if let Ok(k) = serde_json::to_string(&v)
                && let Some(n) = left.get_mut(&k)
                && *n > 0
            {
                *n -= 1;
                continue;
            }
            tail.push(v);
        }
        (tail, head)
    }
}

/// 目的：协作在工作线程上要做的事：处置一张**已经出队**的回答（放行类要接着跑泵），或从断点继续。
///   为什么要分：回答的**落定**（校验 / 出队 / 记账）在核心线程上做完，长流程的处置才在这里跑；
///   两者都不属于对外契约（前端只发"回答"，不问后端怎么推进）。
pub(crate) enum CollabWork {
    /// 目的：处置一张已经出队的回答（队列给的处置票）。
    Dispose(crate::capabilities::session::api::GateTicket),
    Resume,
}

/// 目的：一次"要一个成员回合"的请求：泵在工作线程上让出，回头找主线程驱动（它才拿得到各 agent 的会话）。
pub(crate) struct AskReq {
    pub(crate) agent: String,
    /// 目的：本回合的**身份块**（由泵按当前提示词册现渲染）。
    pub(crate) identity: String,
    /// 目的：本回合的提示（开场词 / 轮转词）。
    pub(crate) turn: Vec<crate::capabilities::llm::api::Msg>,
    /// 目的：工具总表与角色表的能力面（**共享一份**）：发放工具面与校验越权都用它。
    pub(crate) systools: Arc<dyn crate::capabilities::tools::api::Tools>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) opts: crate::capabilities::llm::api::CompleteOpts<'static>,
    /// 目的：这一回合属于第几轮（写进 agent 会话的回合标记）。
    pub(crate) round: usize,
    /// 目的：回合 id（整场工作单调递增；两边对得上就靠它）。
    pub(crate) turn_id: u64,
}

/// 目的：生成前的**准备结果**：能跑（`Run`）、只回事件不跑（`Immediate`）、不是单 agent（`NotSingle`）。
///   由 `Turn::prepare_single` / `prepare_node` 在核心线程上产出，交给工作线程跑模型调用。
pub(crate) enum Prepared {
    Run {
        /// 目的：已从核心表取出的会话（生成期间表里没有它）。
        session: Box<AgentSession>,
        /// 目的：身份块（现渲染）。
        identity: String,
        /// 目的：生成前就要外送的事件（形态变更提示、"先发言再继续"提醒）。
        prefix: Vec<SessionEvent>,
        /// 目的：节点的派发任务（在压缩之后注入；单 agent 发言为 None）。
        task: Option<String>,
        /// 目的：本次调用的通道参数。
        llm: crate::capabilities::llm::api::LlmOpts,
        /// 目的：这一轮的增量落盘手柄。
        persister: Persister,
    },
    Immediate(Vec<SessionEvent>),
    NotSingle,
}

/// 目的：回档重建的**描述**（改需求那条路径要宿主执行重建，本层不持生命周期能力）。
///   `events` 已经是改写后的盘上事件切片；`next` 是沿用"全量最大 id + 1"的续号。
pub(crate) struct RestoreRequest {
    pub(crate) sid: String,
    pub(crate) meta: SessionMeta,
    pub(crate) events: Vec<serde_json::Value>,
    pub(crate) next: u64,
}

/// 目的：**回合推进需要宿主代做的入口**（业务循环自己做不了、必须在核心线程或宿主的线程上做的那几件）。
///   为什么由 turn 声明：循环在 turn 里，根（命令队列手柄）实现它；R12 的端口持有者是 turn。
///   约束：宿主只在此接口内碰核心状态；turn 不认识命令队列、线程池与 guest 机制。
pub trait TurnHost: Send + Sync + Clone + 'static {
    /// 目的：在宿主的核心线程上改 `Turn`（唯一的状态入口）。
    /// 参数：`f` = 要执行的动作，拿 `&mut Turn`。
    /// 返回：动作的返回值；核心队列已断或动作报错时原样返回。
    /// 错误：队列不可用、动作返回的错误文案。
    fn core<T>(
        &self,
        f: impl FnOnce(&mut Turn) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String>
    where
        T: Send + 'static;

    /// 目的：同 `core`，但把登记处与主机探针一并给动作——重建会话、取沙箱、解析调用参数都要它们。
    /// 参数：`f` = 要执行的动作，拿 `&mut Turn` 与两份事实端口。
    /// 返回：动作的返回值。
    /// 错误：队列不可用、动作返回的错误文案。
    fn core_env<T>(
        &self,
        f: impl FnOnce(&mut Turn, &dyn Registry, &dyn HostProbe) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String>
    where
        T: Send + 'static;

    /// 目的：本核心的事件台（多端订阅；生成线程按它外送短暂事件与定稿事件）。
    fn events(&self) -> Arc<EventBus>;

    /// 目的：确保这条会话的 guest 已经起来（单 agent 回合、节点、协作、成员回合共用的兜底）。
    /// 错误：准备启动失败、启动失败或同一台 guest 正在启动。
    fn ensure_guest(&self, sid: &str) -> Result<(), String>;

    /// 目的：按用户「停止」收这个会话（含整棵子树的裁决作废与运行态冻结）。
    /// 约束：返回值是实际停下的会话，本层不需要，宿主自行处理。
    fn stop(&self, sid: &str);

    /// 目的：这个会话的**裁决队**（工具级确认进它）：只有接了交互前端才给（否则这一趟不接确认）。
    /// 错误：取裁决队时队列已断。
    fn decisions_of(&self, sid: &str) -> Result<Option<Arc<dyn Gates>>, String>;

    /// 目的：这个会话的**工具层提问端口**：卡片的登记 → 外送 / 落盘 → 停会话，全走该会话那条道。
    /// 参数：`door` = 该会话的裁决通道；`bus` / `persister` = 外送与落盘。
    fn session_ask(
        &self,
        door: &Arc<dyn Gates>,
        sid: &str,
        bus: Arc<EventBus>,
        persister: Persister,
    ) -> Arc<dyn AskUser>;

    /// 目的：给这条会话注入代理侧的工具执行者（代理会话才需要），把会话原样交回。
    /// 参数：`session` = 这一回合的会话（按值进出，宿主不持有它）；`sid` = 会话名。
    /// 返回：装好执行者的会话；不需要装时原样返回。
    /// 错误：读落盘失败、造执行者失败。
    fn inject_proxy_handler(
        &self,
        session: AgentSession,
        sid: &str,
    ) -> Result<AgentSession, String>;

    /// 目的：登记一次进行中的生成，返回**取消旗标**（停止时由它打断模型调用）。
    fn register_job(&self, sid: &str) -> Arc<AtomicBool>;

    /// 目的：撤销一条登记的生成（无论成功、失败还是崩溃都要走到）。
    fn unregister_job(&self, sid: &str, flag: &Arc<AtomicBool>);

    /// 目的：回答一张裁决卡（**宿主的命令入口**）：先把回答落定，放行类再接着推进；长流程由宿主点火。
    /// 参数：`sid` = 会话；`card` = 卡号；`option` = 选项 id；`note` = 附言。
    /// 返回：推进回执（只带事件台头部序号）。
    /// 错误：没有挂起、卡号不是队首、选项不属于那张卡——原样拒绝。
    fn answer_card(
        &self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<Advance, String>;
}

/// 目的：回合的入站契约：说一句、继续、写需求、回答裁决卡、看裁决队、撤回同意、压缩、在跑吗。
///   宿主（runtime 的 `Handle`）实现它；呈现层按屏幕只取这一面。
pub trait TurnOps: Send + Sync {
    /// 目的：单 agent 会话里说一句（生成可被 `stop` 中止）。
    fn say(&self, sid: &str, text: &str, out: Output) -> Result<Advance, String>;
    /// 目的：继续一次会话：被停止过就先解冻整棵子树再接着走；协作从断点推进，单 agent / 代理补一轮「继续」。
    fn continue_flow(&self, sid: &str, out: Output) -> Result<Advance, String>;
    /// 目的：协作会话：写下本次需求（起点那一关）；其余推进一律走"回答一张卡"。
    fn set_task(&self, sid: &str, text: &str) -> Result<Advance, String>;
    /// 目的：**回答一张裁决卡**（唯一的回答口）：带卡片 id + 选项 id（+ 附言）。
    /// 约束：校验选项 id 属于**当时那张卡**的选项集（防旧卡的答案放行新请求）；不合就如实拒绝。
    /// 与「停止」同一条直路：这一答先落定（出队 + 记一条回答），处置再脱离调用方跑。
    fn answer_card(
        &self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<Advance, String>;
    /// 目的：当前挂着的那一队裁决（None = 没有等你定的事）：队首卡按选项渲染，后面还在等的几张如实列出。
    /// 约束：判据只有这一处——推的事件与快照的 pending 都从它派生（见 docs/session/session-model.md）。
    fn open_queue(&self, sid: &str) -> Result<Option<DecisionQueue>, String>;
    fn withdraw_agree(&self, sid: &str, agent: &str) -> Result<Advance, String>;
    /// 目的：压缩这个会话的上下文（AI 自己压；压不动如实说）。
    fn compact(&self, sid: &str) -> Result<Advance, String>;
    #[allow(dead_code)]
    fn is_running(&self, sid: &str) -> bool;
}
