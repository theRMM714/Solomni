//! 入站契约：呈现层（CLI / Web）与核心之间的唯一通道（见 docs/architecture/contracts.md）。
//!
//! 形态：**命令 + 事件**。
//! - 核心常驻自己的执行线程、独占全部状态：呈现层拿不到 `Core`，也拿不到任何核心锁。
//! - 呈现层只持有 `Ops`：四个**按角色切分**的能力接口 + 一个可订阅的事件台。
//! - 每条命令一次同步回复（std mpsc）：呈现层仍是「调用即拿结果」，不必改写成异步。
//! - **并发归核心**：停止/取消由 `JobRegistry` 承担，不进命令队列——所以生成期间照样立刻生效；
//!   呈现层因此不需要知道「生成时核心状态被占用」这类内部事实。
//! - 事件台由核心独占生产：任何数量的消费者按 `since` 增量取，序号供客户端去重。
//!
//! 这里**不出现 HTTP / JSON 封装 / 路由**：那些是呈现层的传输事（见 presentation/routes.rs）。
//! 事件与读模型（`SessionEvent`、`*View`）是**事实**的线格式，仍归 core。

use crate::core::agents::AgentView;
use crate::core::events::{Live, SessionEvent};
use crate::core::exec::Tier;
use crate::core::history::{HistoryView, SessionMeta};
use crate::core::module::Roster;
use crate::core::providers::{AppSettings, ModelView, ProviderView};
use crate::core::Prepared;
use crate::core::{
    AgentMeta, AgentSuggestion, CollabStep, Core, FilesView, Pending, RuntimeReport, SessionConfig,
    SessionEdit, SessionId, SessionView, WorkMode, WorkOpened, WorkSpec,
};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

// ---------- 输出方式与推进结果 ----------

/// 一次生成的输出方式：流式逐片，还是只要最终结果。
/// 「怎么呈现」是呈现层的决定，所以由调用方显式给出，核心不替它猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Stream,
    Final,
}

/// 一次会话推进的结果：本批事件 + 它在事件台上的序号。
/// 客户端用序号与长轮询去重（同一批事件既随回复返回、也会被其它端从事件台取走）。
#[derive(Debug, Clone)]
pub struct Advance {
    pub events: Vec<SessionEvent>,
    pub seq: u64,
}

// ---------- 事件台 ----------

/// 事件台上的一条：全局序号 + 会话 + 事实。
#[derive(Debug, Clone)]
pub struct EventLine {
    pub seq: u64,
    pub sid: SessionId,
    pub events: Vec<SessionEvent>,
}

/// 事件上限与保留量：本地单机足够，防无界增长。
const BUS_MAX: usize = 10_000;
const BUS_KEEP: usize = 5_000;

/// 事件发布台：核心是唯一生产者（`push` 只由本模块调用）；
/// 任何数量的消费者按 `since` 增量取——多端并用、断线重连都不用额外机制。
pub struct EventBus {
    inner: Mutex<BusInner>,
}

#[derive(Default)]
struct BusInner {
    seq: u64,
    lines: Vec<EventLine>,
}

impl EventBus {
    pub fn new() -> Arc<EventBus> {
        Arc::new(EventBus {
            inner: Mutex::new(BusInner::default()),
        })
    }

    /// 入箱一批事实，返回它的全局序号。核心内部调用；锁中毒不算致命（拿回内层继续）。
    fn push(&self, sid: &str, events: &[SessionEvent]) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
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

    /// 测试专用：往事件台放一条空事件批，让长轮询立刻返回（不真等 20 秒）。
    #[cfg(test)]
    pub(crate) fn seed_for_test(&self, sid: &str) -> u64 {
        self.push(sid, &[])
    }

    /// 取 `since` 之后的事件批 + 当前头部（**同一把锁内**：客户端据头部推进游标不会漏事件）。
    pub fn snapshot(&self, sid: Option<&str>, since: u64) -> (Vec<EventLine>, u64) {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let lines = g
            .lines
            .iter()
            .filter(|l| l.seq > since && sid.is_none_or(|x| l.sid == x))
            .cloned()
            .collect();
        (lines, g.seq)
    }
}

// ---------- 取消注册表（并发归核心） ----------

/// 生成中作业的取消表。核心登记，呈现层只能说「停哪个会话」。
/// 「停止」**不排队、不碰核心状态**，所以生成期间也能立刻生效——这是它存在的全部理由。
#[derive(Default)]
pub struct JobRegistry {
    running: Mutex<HashMap<SessionId, Arc<AtomicBool>>>,
}

impl JobRegistry {
    pub fn new() -> Arc<JobRegistry> {
        Arc::new(JobRegistry::default())
    }

    /// 登记一个生成中作业并给出它的取消标志（核心内部用）。
    fn register(&self, sid: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sid.to_string(), Arc::clone(&flag));
        flag
    }

    fn unregister(&self, sid: &str) {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(sid);
    }

    /// 请求停止该会话正在跑的生成；返回是否确实有一个在跑。
    pub fn stop(&self, sid: &str) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .cloned()
        {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// 该会话是否正在生成（配置界面据此拒绝改到一半的语义）。
    pub fn is_running(&self, sid: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(sid)
    }
}

// ---------- 按角色切分的能力接口 ----------

/// 会话能力：创建、推进、回档、编辑、停止。会话界面只需要这一个。
pub trait SessionOps: Send + Sync {
    fn create_work(&self, spec: WorkSpec) -> Result<WorkOpened, String>;
    /// 单 agent 会话里说一句（生成可被 `stop` 中止）。
    fn say(&self, sid: &str, text: &str, out: Output) -> Result<Advance, String>;
    /// 继续一次会话（协作的执行阶段 / 单 agent 的继续）。
    fn continue_flow(&self, sid: &str, out: Output) -> Result<Advance, String>;
    /// 协作推进到下一个阶段（task / slate / begin / answer）。
    fn collab_step(&self, sid: &str, step: CollabStep, text: &str) -> Result<Advance, String>;
    fn withdraw_agree(&self, sid: &str, agent: &str) -> Result<Advance, String>;
    /// 核心按模块名给出的名单草案（协作代拟名单）。
    fn slate(&self, sid: &str) -> Result<Vec<AgentMeta>, String>;
    /// 回档：返回重放后的完整事件流（已是线格式，供前端整体重建）。
    fn rewind(&self, sid: &str, keep_id: u64) -> Result<Vec<serde_json::Value>, String>;
    /// 压缩这个会话的上下文（AI 自己压；压不动如实说）。
    fn compact(&self, sid: &str) -> Result<Advance, String>;
    /// 改需求：同样返回完整重放。
    fn update_task(&self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String>;
    fn pending(&self, sid: &str) -> Result<Option<Pending>, String>;
    fn config(&self, sid: &str) -> Result<SessionConfig, String>;
    fn edit(&self, sid: &str, edit: SessionEdit) -> Result<(), String>;
    /// 投喂文件进本次工作的 work/；返回 false = 同名已存在（由用户决定覆盖或改名）。
    fn upload(&self, sid: &str, name: &str, bytes: &[u8], overwrite: bool) -> Result<bool, String>;
    fn files(&self, sid: &str) -> Result<FilesView, String>;
    fn exists(&self, sid: &str) -> Result<bool, String>;
    /// 请求停止该会话在跑的生成；返回是否确实有一个在跑。
    fn stop(&self, sid: &str) -> bool;
    fn is_running(&self, sid: &str) -> bool;
}

/// 登记处能力：供应商 / 模型 / agent / 基本设置，以及通道上的模型发现。
pub trait RegistryOps: Send + Sync {
    fn providers(&self) -> Result<Vec<ProviderView>, String>;
    fn upsert_provider(&self, id: &str, base_url: &str, api_key: &str) -> Result<(), String>;
    fn remove_provider(&self, id: &str) -> Result<bool, String>;
    fn models(&self) -> Result<Vec<ModelView>, String>;
    fn core_model(&self) -> Result<Option<String>, String>;
    fn upsert_model(
        &self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
        // 上下文窗口（tokens）；0 = 保留现值（新建缺省 32k）。
        context: u64,
    ) -> Result<(), String>;
    fn remove_model(&self, id: &str) -> Result<bool, String>;
    fn set_core_model(&self, id: &str) -> Result<bool, String>;
    fn agents(&self) -> Result<Vec<AgentView>, String>;
    fn upsert_agent(
        &self,
        name: &str,
        modules: &[String],
        model: &str,
        note: &str,
    ) -> Result<(), String>;
    fn remove_agent(&self, name: &str) -> Result<bool, String>;
    fn settings(&self) -> Result<AppSettings, String>;
    fn set_settings(&self, app: AppSettings) -> Result<(), String>;
    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String>;
    /// 实测一条通道支不支持原生工具调用（要真实网络；三种结论都如实回报，
    /// 只把**确定**的结论写回登记处 —— 这条规则在 core，不在呈现层）。
    fn probe_model_tools(&self, id: &str) -> Result<crate::core::ports::ProbeOutcome, String>;
    /// 实测这种"回放形状"供应商收不收、模型有没有真的读懂（要真实网络；**不改登记处**）。
    fn probe_replay_shape(&self, id: &str) -> Result<crate::core::providers::ReplayReport, String>;
}

/// 历史能力：落盘会话的列表 / 打开 / 删除，以及在世会话与历史合并后的总览。
pub trait HistoryOps: Send + Sync {
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String>;
}

/// 发现能力：清单与能力事实（都只报事实，不做选择）。
pub trait DiscoveryOps: Send + Sync {
    fn roster(&self) -> Result<Roster, String>;
    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String>;
    /// 核心按任务推荐的 agent 草案（带理由；用户可改）。
    fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String>;
}

// ---------- 核心手柄（命令通道） ----------

/// 协作在工作线程上要做的事：推进一个阶段，或从断点继续。
/// 为什么要分开：`CollabStep` 是**对外**的阶段枚举（前端按 pending 决定），
/// "继续"不是它的阶段之一（前端走 `continue_flow`），所以内部再分一层，不污染对外契约。
#[derive(Clone, Copy)]
enum CollabWork {
    Step(CollabStep),
    Resume,
}

/// 一次命令：在核心自己的线程上执行（因此核心状态不需要任何锁）。
type Job = Box<dyn FnOnce(&mut Core) + Send>;

/// 核心手柄：呈现层用它发命令。克隆廉价（内部只是通道 + 两个共享句柄）。
#[derive(Clone)]
pub struct CoreHandle {
    tx: Sender<Job>,
    jobs: Arc<JobRegistry>,
    bus: Arc<EventBus>,
}

/// 一次"要一个成员回合"的请求：泵在工作线程上让出，回头找主线程驱动（它才拿得到各 agent 的会话）。
struct AskReq {
    agent: String,
    /// 本回合的**身份块**（由泵按当前提示词册现渲染）。
    identity: String,
    /// 本回合的提示（开场词 / 轮转词）。
    turn: Vec<crate::core::ports::Msg>,
    /// 角色表（按值带一份小表）：发放工具面与校验越权都用它。
    systools: crate::core::roles::SystemTools,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    opts: crate::core::ports::CompleteOpts<'static>,
    /// 这一回合属于第几轮（写进 agent 会话的回合标记）。
    round: usize,
    /// 回合 id（整场工作单调递增；两边对得上就靠它）。
    turn_id: u64,
}

impl CoreHandle {
    /// 把核心搬到它自己的执行线程：此后所有核心状态只被这一个线程碰。
    /// 全部手柄丢弃后线程自然结束（通道断开即退出）。
    pub fn spawn(core: Core) -> Result<CoreHandle, String> {
        let worker_log = core.log_handle();
        let jobs = JobRegistry::new();
        let bus = EventBus::new();
        let (tx, rx) = mpsc::channel::<Job>();
        let handle = CoreHandle { tx, jobs, bus };
        // 注意：工作线程**绝不能**捕获取手柄（那会持有一个 Sender，通道永不闭合、线程永不退出）。
        std::thread::Builder::new()
            .name("solomni-core".to_string())
            .spawn(move || {
                let mut core = core;
                while let Ok(job) = rx.recv() {
                    // 一条命令 panic 不该带走整个核心：接住并继续（回包通道随闭包销毁，调用方会看到「无回应」）。
                    if catch_unwind(AssertUnwindSafe(|| job(&mut core))).is_err() {
                        worker_log.error(
                            "core::api",
                            "核心命令 panic：已接住，核心继续服务（请查上面的 panic 现场）",
                        );
                    }
                }
                worker_log.info("core::api", "核心线程退出：全部手柄已释放");
            })
            .map_err(|e| format!("启动核心线程失败：{}", e))?;
        Ok(handle)
    }

    /// 发一条命令并等回复：呈现层因此仍是「调用即拿结果」，不需要改成异步。
    fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Core) -> Result<T, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |core: &mut Core| {
            let _ = tx.send(f(core));
        });
        self.tx
            .send(job)
            .map_err(|_| "核心已停止：无法发送命令".to_string())?;
        rx.recv()
            .map_err(|_| "核心无回应：命令执行中发生 panic，或核心线程已停止".to_string())?
    }

    /// 本核心的事件台（多端订阅）。
    pub fn events(&self) -> Arc<EventBus> {
        Arc::clone(&self.bus)
    }

    /// 测试专用：注入一条必定 panic 的命令，验证「一条命令 panic 不带垮整个核心」。
    #[cfg(test)]
    pub(crate) fn panic_probe(&self) -> Result<(), String> {
        self.call(|_core| -> Result<(), String> { panic!("入站契约测试注入的 panic") })
    }

    /// 单 agent 生成：**核心队列只占两步短命令**（取出会话 / 交回会话），生成本身在工作线程上跑。
    /// 为什么必须这样：生成要跑几十秒到几分钟，以前它占着唯一的命令队列，
    /// 读接口（历史列表 / 状态）与其它会话的命令全排在它后面——界面因此"假死"。
    /// 不变量：会话状态只被一个线程碰——生成期间由工作线程独占，核心表里只留"生成中"这一态。
    fn single_generation(
        &self,
        sid: &str,
        text: Option<String>,
        out: Output,
    ) -> Result<Advance, String> {
        // ① 短命令：检查 + 把会话取出来 + 定下本次调用参数（都在核心线程上，毫秒级）。
        let prepared = self.call({
            let sid = sid.to_string();
            let t = text.clone();
            move |core| core.prepare_single(&sid, t.as_deref(), out == Output::Stream)
        })?;
        self.run_prepared(sid, prepared, text)
    }

    /// 一个节点的执行回合：**核心把任务提示词作为系统消息注入**（不是用户发言），再生成。
    /// 与 CLI 的 `Core::drive_node` 同一条语义——各前端只做各自的界面，管道只有这一条。
    fn node_generation(&self, child: &str, objective: &str) -> Result<Advance, String> {
        let prepared = self.call({
            let (child, objective) = (child.to_string(), objective.to_string());
            move |core| core.prepare_node(&child, &objective)
        })?;
        self.run_prepared(child, prepared, None)
    }

    /// 生成的工作线程与收尾：用户发言、**核心注入的任务**、继续都走这一条。
    fn run_prepared(
        &self,
        sid: &str,
        prepared: Prepared,
        text: Option<String>,
    ) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        let (session, identity, prefix, llm, persister) = match prepared {
            // 不用跑模型（例如"末条是 AI 发言"）：把提示直接回给调用方。
            Prepared::Immediate(events) => {
                let seq = bus.push(sid, &events);
                return Ok(Advance { events, seq });
            }
            // 协作会话的"继续"仍在核心线程上推进（B-1 只搬单 agent 生成）。
            Prepared::NotSingle => {
                if text.is_some() {
                    return Err("该会话不是单 agent 模式".to_string());
                }
                // 协作会话的"继续"：走同一条 own-and-return（泵在工作线程上）。
                return self.collab_generation(sid, CollabWork::Resume, "");
            }
            Prepared::Run {
                session,
                identity,
                prefix,
                llm,
                persister,
            } => (*session, identity, prefix, llm, persister),
        };
        // 取消标志在**派发时**就登记：生成一开始「停止」就能生效（它本来就不进队列）。
        let cancel = jobs.register(sid);
        // ② 工作线程：跑生成。短暂事件（流式增量 / 工具行）直送事件台——它是独立锁，不进核心队列。
        let worker = {
            let sid = sid.to_string();
            let bus = Arc::clone(&bus);
            std::thread::Builder::new()
                .name("solomni-gen".to_string())
                .spawn(move || {
                    let mut session = session;
                    let mut emit = {
                        let bus = Arc::clone(&bus);
                        let sid = sid.clone();
                        move |ev: SessionEvent| {
                            bus.push(&sid, std::slice::from_ref(&ev));
                        }
                    };
                    let mut live = Live {
                        llm,
                        cancel,
                        emit: &mut emit,
                    };
                    // 逐轮外送 + 边落盘：一轮跑完就上屏并落盘（中途刷新页面因此看得到已产生的部分）。
                    // seq 取**最后一次**入台的序号：客户端按它去重（逐轮外送因此是多批）。
                    let mut events: Vec<SessionEvent> = Vec::new();
                    let mut seq = 0u64;
                    let mut sink = |ev: SessionEvent| {
                        seq = bus.push(&sid, std::slice::from_ref(&ev));
                        if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                            bus.push(&sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
                        }
                        events.push(ev);
                    };
                    // 生成前的提示（例如工具形态变更）先出，再跑。
                    for ev in prefix {
                        sink(ev);
                    }
                    match &text {
                        Some(t) => session.say(t, &identity, &mut live, &mut sink),
                        None => session.continue_reply(&identity, &mut live, &mut sink),
                    }
                    (session, events, seq)
                })
                .map_err(|e| format!("起生成线程失败：{}", e))?
        };
        // ③ 收尾：拿回会话 → 入台 → 交回核心（重新插入 + 转录落盘）。
        //    **所有状态变更仍只发生在核心线程上**：工作线程只跑生成，不碰核心状态。
        let joined = worker.join();
        jobs.unregister(sid);
        let (session, events, seq) = match joined {
            Ok(x) => x,
            Err(_) => {
                // 线程崩了：会话对象随线程没了，但**转录在盘上**——解除"生成中"，
                // 下次访问按落盘转录重建。绝不把会话卡在"生成中"。
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("生成线程崩溃：会话已按落盘转录保留，可继续".to_string());
            }
        };
        // 事件已由上面的 sink 逐轮入台（不再整批补推，否则同一批事实在台上有两份）。
        // 交回核心只做"重新插入"：转录也已逐轮增量落盘。
        let parent = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.put_single(&sid, session))
        })?;
        // 这是**子会话**完成：叫醒父会话推进任务链（脱离本次调用，不等它跑完）。
        if let Some(parent) = parent {
            self.spawn_detached_collab(&parent);
        }
        Ok(Advance { events, seq })
    }

    /// 主线程侧：为一个成员回合取该 agent 的会话，跑完把回合结果带回来（并落进它自己的会话）。
    fn run_member_turn(
        &self,
        sid: &str,
        req: &AskReq,
    ) -> Result<crate::core::engine::MemberTurn, String> {
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
        let session = self.call({
            let name = child.clone();
            move |core| core.take_single(&name)
        })?;
        let systools = req.systools.clone();
        let cancel = std::sync::Arc::clone(&req.cancel);
        let opts = req.opts;
        let turn = req.turn.clone();
        let identity = req.identity.clone();
        let agent = req.agent.clone();
        let _ = &turn;
        let joined = std::thread::Builder::new()
            .name("solomni-member".to_string())
            .spawn(move || {
                let mut s = session;
                let ran = {
                    // 这一回合的消息 = 身份块（现渲染）+ 本回合工具 + 该会话的**对话** + 开场/轮转词。
                    let hist = s.dialogue().to_vec();
                    let (chat, tools) = s.parts_mut();
                    crate::core::engine::Discussion::turn_with(
                        &systools,
                        "discussant",
                        &cancel,
                        opts,
                        &agent,
                        &identity,
                        &hist,
                        chat,
                        tools,
                        turn,
                        &mut |_e| {},
                    )
                };
                (s, ran)
            })
            .map_err(|e| format!("起成员线程失败：{}", e))?
            .join();
        let (mut s, ran) = match joined {
            Ok(x) => x,
            Err(_) => {
                self.call({
                    let name = child.clone();
                    move |core| {
                        core.abort_running(&name);
                        Ok(())
                    }
                })?;
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
                return Err(err);
            }
        };
        // 该回合的产出落进**它自己的会话**：回合标记 + 核实行 + 它自己的发言。
        // 没表态也要落它自己的会话（那是它的回合记录）；标签如实写"未表态"。
        let tag = match turn.verb {
            Some(crate::core::envelope::Verb::Say) => "say",
            Some(crate::core::envelope::Verb::Ask) => "ask",
            Some(crate::core::envelope::Verb::Leave) => "leave",
            Some(crate::core::envelope::Verb::Agree) => "agree",
            Some(crate::core::envelope::Verb::Tool) => "tool",
            None => "未表态",
        };
        let note = s.note_turn(req.round, req.turn_id, tag, &turn.text, &turn.lines);
        self.call({
            let name = child.clone();
            move |core| {
                core.put_single(&name, s);
                core.persister(&name).persist(&note);
                Ok(())
            }
        })?;
        Ok(turn)
    }

    /// 手动压缩（`/compact`）：**让 AI 自己压**——核心只给 compact 工具，不是系统替它总结。
    /// 与单 agent 同一条 own-and-return：队列只占"取/交"，模型调用在工作线程上（界面不被阻塞）。
    /// 压不动就**如实说**（通知 + 继续用完整上下文），不静默降级、不假装压过。
    pub fn compact(&self, sid: &str) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        let session = self.call({
            let sid = sid.to_string();
            move |core| core.take_single(&sid)
        })?;
        let (prompt, decl, up_to, identity) = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.compact_plan(&sid))
        })?;
        let joined = std::thread::Builder::new()
            .name("solomni-compact".to_string())
            .spawn(move || {
                let mut s = session;
                let made = s.compact_turn(&prompt, decl.as_ref(), &identity);
                if let Ok(summary) = &made {
                    s.compact(up_to, summary);
                }
                (s, made)
            })
            .map_err(|e| format!("起压缩线程失败：{}", e))?
            .join();
        let (s, made) = match joined {
            Ok(x) => x,
            Err(_) => {
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("压缩线程崩溃：会话已按落盘转录保留".to_string());
            }
        };
        let summary = match made {
            Ok(sm) => sm,
            Err(err) => {
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.put_single(&sid, s);
                        Ok(())
                    }
                })?;
                let note = SessionEvent::Notice(crate::core::events::interrupted_note(&format!(
                    "压缩没成功：{}",
                    err
                )));
                let seq = bus.push(sid, std::slice::from_ref(&note));
                return Ok(Advance {
                    events: vec![note],
                    seq,
                });
            }
        };
        let ev = SessionEvent::Compacted { up_to, summary };
        self.call({
            let sid = sid.to_string();
            let ev = ev.clone();
            move |core| {
                core.put_single(&sid, s);
                core.persister(&sid).persist(std::slice::from_ref(&ev));
                Ok(())
            }
        })?;
        let seq = bus.push(sid, std::slice::from_ref(&ev));
        Ok(Advance {
            events: vec![ev],
            seq,
        })
    }

    /// 起一轮**脱离调用方**的节点执行：核心注入任务 + 不等它跑完。
    /// 完成后由叫醒逻辑推进父会话——所以这里只是"点火"。
    fn spawn_detached_node(&self, sid: &str, objective: &str) {
        // 节点执行也用整场工作的同一套回合计数（回档同步靠两边同一套编号）。
        let _ = self.call({
            let sid = sid.to_string();
            move |core| {
                core.bump_turn_of_child(&sid);
                Ok(())
            }
        });
        let me = self.clone();
        let (sid, objective) = (sid.to_string(), objective.to_string());
        let _ = std::thread::Builder::new()
            .name("solomni-node".to_string())
            .spawn(move || {
                let _ = me.node_generation(&sid, &objective);
            });
    }

    /// 起一次**脱离调用方**的协作推进（叫醒父会话用）：不等它跑完。
    fn spawn_detached_collab(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-chain".to_string())
            .spawn(move || {
                let _ = me.collab_generation(&sid, CollabWork::Resume, "");
            });
    }

    /// 协作的长步骤（开始讨论 / 回答 / 继续）：与单 agent 同一条 own-and-return——
    /// 队列只占"取/交"两步，泵在工作线程上跑；事件**边产边送**事件台，界面因此能看着讨论推进。
    ///
    /// **核心驱动**（见 docs/architecture/session-model.md 二之二）：泵只决定"该问谁"，
    /// 成员回合由主线程取该 agent 的会话去跑（它才拿得到那些会话）。所以泵线程与主线程**握手**：
    /// 泵让出 → 发 AskReq → 主线程跑完回 MemberTurn → 泵继续。
    fn collab_generation(
        &self,
        sid: &str,
        work: CollabWork,
        text: &str,
    ) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        // **先登记再取会话**：登记早于派发，所以「停止」从派发那一刻起就能生效。
        let cancel = jobs.register(sid);
        let session = match self.call({
            let sid = sid.to_string();
            move |core| core.take_collab(&sid)
        }) {
            Ok(s) => s,
            Err(e) => {
                jobs.unregister(sid);
                return Err(e);
            }
        };
        // 把「停止」接到泵上：它在每次模型调用前与**调用中途**都看这个标志。
        let mut session = session;
        session.set_cancel(Arc::clone(&cancel));
        // 增量落盘手柄：泵产出一条定稿事件就落一条，中途刷新页面因此能看到已产生的部分。
        let persister = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.persister(&sid))
        })?;
        let text = text.to_string();
        // 提醒上限由设置来（用户可调，见 session-model.md 二）：起线程前问一次核心。
        // 调用次数**没有上限**：模型继续核实就继续跑，直到它给出表态（或用户点停止）。
        let remind_cap = self.call(|core| Ok(core.discuss_remind_cap())).unwrap_or(3);
        let handle = self.clone();
        // 握手通道：泵 → 主线程（要一个成员回合）；主线程 → 泵（回合结果）。
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<AskReq>();
        let (turn_tx, turn_rx) =
            std::sync::mpsc::channel::<Result<crate::core::engine::MemberTurn, String>>();
        let worker = {
            let sid = sid.to_string();
            let bus = Arc::clone(&bus);
            std::thread::Builder::new()
                .name("solomni-collab".to_string())
                .spawn(move || {
                    let mut c = session;
                    let mut events: Vec<SessionEvent> = Vec::new();
                    let mut seq = 0u64;
                    // 安全网计数（见循环尾）：提醒/重问必须有终点，不能让泵空转。
                    let mut guard = 0usize;
                    {
                        // 边产边送 + 边落盘：长流程里用户能看着讨论一轮轮推进，
                        // 中途刷新页面也能看到已产生的部分（不再等整段结束才一次性出现）。
                        let mut sink = |ev: SessionEvent| {
                            seq = bus.push(&sid, std::slice::from_ref(&ev));
                            // 落盘失败要**如实告知**（落一条警告进事件台），不静默丢历史。
                            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                                bus.push(&sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
                            }
                            events.push(ev);
                        };
                        match work {
                            CollabWork::Step(CollabStep::Begin) => {
                                c.begin(text.contains("allow"), &mut sink)
                            }
                            CollabWork::Step(CollabStep::Answer) => c.answer(&text, &mut sink),
                            // 审查关卡点「同意」：记下过关，然后接着推进（这条是生产路径——
                            // 只接 Core::collab_continue 会漏掉它，单元测试看不出来，e2e 才逮得到）。
                            CollabWork::Step(CollabStep::ApprovePlan) => {
                                c.approve_plan(&mut sink);
                                c.resume(&mut sink);
                            }
                            CollabWork::Step(_) => {}
                            CollabWork::Resume => c.resume(&mut sink),
                        }
                        // 核心驱动：泵让出"该问谁"就回头找主线程（它才拿得到各 agent 的会话）。
                        loop {
                            // 先看有没有已经让出的那一步；没有就推一步（推完再看一次）。
                            let ask = match c.take_ask() {
                                Some(a) => Some(a),
                                None => {
                                    c.pump_with(&mut sink);
                                    c.take_ask()
                                }
                            };
                            let Some((i, identity, turn)) = ask else { break };
                            let Some(agent) = c.member_id(i) else { break };
                            // 提醒时要往它自己的会话里写（那边用的是同一个名字）。
                            let agent_name = agent.clone();
                            let req = AskReq {
                                agent,
                                identity,
                                turn,
                                systools: c.systools().clone(),
                                cancel: c.disc_cancel(),
                                opts: c.disc_opts(),
                                round: c.round(),
                                turn_id: c.next_turn_id(),
                            };
                            let turn_id = req.turn_id;
                            if ask_tx.send(req).is_err() {
                                break;
                            }
                            // 等主线程跑完这一回合（它取会话、跑模型、落盘，再把结果送回来）。
                            let Ok(res) = turn_rx.recv() else { break };
                            match res {
                                Ok(turn) => {
                                    // **核心只提醒、不强制**（见 session-model.md 二）：
                                    // 没表态时按计数决定"注入提醒后重问"还是"记未回应后放过"。
                                    let after = c.after_member_turn(
                                        i,
                                        turn.verb.is_some(),
                                        c.cancelled(),
                                        remind_cap,
                                    );
                                    match after {
                                        crate::core::engine::AfterTurn::Done => {
                                            c.feed_with(i, turn, turn_id, &mut sink)
                                        }
                                        crate::core::engine::AfterTurn::Remind => {
                                            // 提醒进**它自己的会话**（系统消息）；不 feed——泵重问同一个人。
                                            let text = c.reminder_text();
                                            let child =
                                                format!("{}--{}", sid, agent_name);
                                            let target = child.clone();
                                            if let Ok(evs) = handle.call(move |core| {
                                                core.note_system(&child, &text)
                                            }) {
                                                // 提醒属于**它自己的会话**：按子会话的 sid 外送，
                                                // 不能混进主会话的事件流（否则主会话会冒出系统行）。
                                                for e in evs {
                                                    bus.push(&target, std::slice::from_ref(&e));
                                                }
                                            }
                                        }
                                        crate::core::engine::AfterTurn::Unanswered => {
                                            c.pass_over(i, &mut sink)
                                        }
                                    }
                                    // 安全网：提醒/重问必须有终点（计数有上限，这里再兜一层）。
                                    guard += 1;
                                    if guard > 500 {
                                        sink(SessionEvent::Notice(
                                            "[警告] 讨论推进次数异常（已到安全上限），已停下等用户处理"
                                                .to_string(),
                                        ));
                                        break;
                                    }
                                }
                                Err(err) => {
                                    // 如实交回（由泵统一外送中断通知），不再往下推。
                                    c.note_turn_failure(err);
                                    c.pump_with(&mut sink);
                                    break;
                                }
                            }
                        }
                    }
                    (c, events, seq)
                })
                .map_err(|e| format!("起协作线程失败：{}", e))?
        };
        // 主线程驱动每个请求：取该 agent 的会话、跑这一回合、把结果发回泵。
        // 模型调用在成员线程上（own-and-return），核心队列只占"取/交"两步——界面因此不被阻塞。
        while let Ok(req) = ask_rx.recv() {
            match self.run_member_turn(sid, &req) {
                Ok(turn) => {
                    if turn_tx.send(Ok(turn)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    // 把失败交回泵（它统一外送中断通知），不再往下推。
                    let _ = turn_tx.send(Err(err));
                    break;
                }
            }
        }
        drop(turn_tx);
        let joined = worker.join();
        jobs.unregister(sid);
        let (c, mut events, mut seq) = match joined {
            Ok(x) => x,
            Err(_) => {
                // 线程崩了：会话对象没了，但转录在盘上——解除"生成中"，下次访问按盘重建。
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("协作线程崩溃：会话已按落盘转录保留，可继续".to_string());
            }
        };
        // 交回核心：重新插入 + **为就绪节点派发子会话**（返回派发事件与待起生成的节点）。
        // 转录已由上面的 sink 增量落盘，这里不再重复落。
        let (spawned, todo) = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.put_collab(&sid, c))
        })?;
        for ev in spawned {
            seq = bus.push(sid, std::slice::from_ref(&ev));
            events.push(ev);
        }
        // 派发：每个就绪节点在**它自己的子会话**里起一轮生成（脱离本次调用，不等它跑完）。
        for (_node, child, objective) in todo {
            self.spawn_detached_node(&child, &objective);
        }
        Ok(Advance { events, seq })
    }
}

impl SessionOps for CoreHandle {
    fn create_work(&self, spec: WorkSpec) -> Result<WorkOpened, String> {
        self.call(move |core| core.create_work(spec))
    }

    fn say(&self, sid: &str, text: &str, out: Output) -> Result<Advance, String> {
        self.single_generation(sid, Some(text.to_string()), out)
    }

    fn continue_flow(&self, sid: &str, out: Output) -> Result<Advance, String> {
        self.single_generation(sid, None, out)
    }

    fn collab_step(&self, sid: &str, step: CollabStep, text: &str) -> Result<Advance, String> {
        match step {
            // 短步骤（写需求 / 定名单）不调模型，而且"定名单"还有落盘与建沙箱的后续——留在核心线程上。
            CollabStep::SetTask | CollabStep::ConfirmSlate => {
                let sid = sid.to_string();
                let text = text.to_string();
                let bus = Arc::clone(&self.bus);
                self.call(move |core| {
                    let events = core.collab_continue(&sid, step, &text)?;
                    let seq = bus.push(&sid, &events);
                    Ok(Advance { events, seq })
                })
            }
            // 长步骤（开始讨论 / 回答）：队列只占"取/交"两步，泵在工作线程上跑。
            // 「同意方案」也要跑泵（过关后接着推进），所以和长步骤走同一条路。
            CollabStep::Begin | CollabStep::Answer | CollabStep::ApprovePlan => {
                self.collab_generation(sid, CollabWork::Step(step), text)
            }
        }
    }

    fn withdraw_agree(&self, sid: &str, agent: &str) -> Result<Advance, String> {
        let sid = sid.to_string();
        let agent = agent.to_string();
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let events = core.withdraw_agree(&sid, &agent)?;
            let seq = bus.push(&sid, &events);
            Ok(Advance { events, seq })
        })
    }

    fn slate(&self, sid: &str) -> Result<Vec<AgentMeta>, String> {
        let sid = sid.to_string();
        self.call(move |core| core.collab_slate(&sid))
    }

    fn compact(&self, sid: &str) -> Result<Advance, String> {
        CoreHandle::compact(self, sid)
    }

    fn rewind(&self, sid: &str, keep_id: u64) -> Result<Vec<serde_json::Value>, String> {
        let sid = sid.to_string();
        self.call(move |core| core.rewind(&sid, keep_id))
    }

    fn update_task(&self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String> {
        let sid = sid.to_string();
        let text = text.to_string();
        self.call(move |core| core.update_task(&sid, &text))
    }

    fn pending(&self, sid: &str) -> Result<Option<Pending>, String> {
        let sid = sid.to_string();
        self.call(move |core| core.collab_pending(&sid))
    }

    fn config(&self, sid: &str) -> Result<SessionConfig, String> {
        let sid = sid.to_string();
        self.call(move |core| core.session_config(&sid))
    }

    fn edit(&self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        let sid = sid.to_string();
        self.call(move |core| core.edit_session(&sid, edit))
    }

    fn upload(&self, sid: &str, name: &str, bytes: &[u8], overwrite: bool) -> Result<bool, String> {
        let sid = sid.to_string();
        let name = name.to_string();
        let bytes = bytes.to_vec();
        self.call(move |core| core.work_upload(&sid, &name, &bytes, overwrite))
    }

    fn files(&self, sid: &str) -> Result<FilesView, String> {
        let sid = sid.to_string();
        self.call(move |core| core.files_view(&sid))
    }

    fn exists(&self, sid: &str) -> Result<bool, String> {
        let sid = sid.to_string();
        self.call(move |core| Ok(core.session_exists(&sid)))
    }

    /// 停止**不走命令队列**：直接置位取消标志，所以生成期间照样立刻生效。
    fn stop(&self, sid: &str) -> bool {
        self.jobs.stop(sid)
    }

    fn is_running(&self, sid: &str) -> bool {
        self.jobs.is_running(sid)
    }
}

impl RegistryOps for CoreHandle {
    fn providers(&self) -> Result<Vec<ProviderView>, String> {
        self.call(|core| Ok(core.provider_views()))
    }
    fn upsert_provider(&self, id: &str, base_url: &str, api_key: &str) -> Result<(), String> {
        let (id, base_url, api_key) = (id.to_string(), base_url.to_string(), api_key.to_string());
        self.call(move |core| core.provider_upsert(&id, &base_url, &api_key))
    }
    fn remove_provider(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.provider_remove(&id))
    }
    fn models(&self) -> Result<Vec<ModelView>, String> {
        self.call(|core| Ok(core.model_views()))
    }
    fn core_model(&self) -> Result<Option<String>, String> {
        self.call(|core| Ok(core.core_model()))
    }
    fn upsert_model(
        &self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
        context: u64,
    ) -> Result<(), String> {
        let (id, name, api_model, provider, note) = (
            id.to_string(),
            name.to_string(),
            api_model.to_string(),
            provider.to_string(),
            note.to_string(),
        );
        self.call(move |core| core.model_upsert(&id, &name, &api_model, &provider, &note, context))
    }
    fn remove_model(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.model_remove(&id))
    }
    fn set_core_model(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.core_set_model(&id))
    }
    fn agents(&self) -> Result<Vec<AgentView>, String> {
        self.call(|core| Ok(core.agent_views()))
    }
    fn upsert_agent(
        &self,
        name: &str,
        modules: &[String],
        model: &str,
        note: &str,
    ) -> Result<(), String> {
        let (name, modules, model, note) = (
            name.to_string(),
            modules.to_vec(),
            model.to_string(),
            note.to_string(),
        );
        self.call(move |core| core.agent_upsert(&name, &modules, &model, &note))
    }
    fn remove_agent(&self, name: &str) -> Result<bool, String> {
        let name = name.to_string();
        self.call(move |core| core.agent_remove(&name))
    }
    fn settings(&self) -> Result<AppSettings, String> {
        self.call(|core| Ok(core.app_settings()))
    }
    fn set_settings(&self, app: AppSettings) -> Result<(), String> {
        self.call(move |core| core.set_app_settings(app))
    }
    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let provider_id = provider_id.to_string();
        self.call(move |core| core.discover_models(&provider_id))
    }
    fn probe_model_tools(&self, id: &str) -> Result<crate::core::ports::ProbeOutcome, String> {
        let id = id.to_string();
        self.call(move |core| core.probe_model_tools(&id))
    }
    fn probe_replay_shape(&self, id: &str) -> Result<crate::core::providers::ReplayReport, String> {
        let id = id.to_string();
        self.call(move |core| core.probe_replay_shape(&id))
    }
}

impl HistoryOps for CoreHandle {
    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.call(|core| Ok(core.history_list()))
    }
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let name = name.to_string();
        self.call(move |core| core.history_open(&name))
    }
    fn delete(&self, name: &str) -> Result<bool, String> {
        let name = name.to_string();
        self.call(move |core| core.history_delete(&name))
    }
    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String> {
        let history = history.to_vec();
        self.call(move |core| Ok(core.session_views(&history)))
    }
}

impl DiscoveryOps for CoreHandle {
    fn roster(&self) -> Result<Roster, String> {
        self.call(|core| Ok(core.scan()))
    }
    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String> {
        self.call(move |core| Ok(core.runtime_report(tier)))
    }
    fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String> {
        let task = task.to_string();
        self.call(move |core| core.suggest_models(&task, mode))
    }
}

// ---------- 入站能力面（组合根装配一次，按需交给呈现层） ----------

/// 入站能力面：四个角色接口 + 事件台。克隆廉价；呈现层只依赖它需要的字段。
/// 契约测试可以用假实现替换任意一个字段——这正是「按角色切分」换来的可测性。
#[derive(Clone)]
pub struct Ops {
    pub sessions: Arc<dyn SessionOps + Send + Sync>,
    pub registry: Arc<dyn RegistryOps + Send + Sync>,
    pub history: Arc<dyn HistoryOps + Send + Sync>,
    pub discovery: Arc<dyn DiscoveryOps + Send + Sync>,
    pub events: Arc<EventBus>,
}

impl Ops {
    /// 组合根用：把同一个核心手柄按角色拆成四个接口（同一个执行线程、同一个事件台）。
    pub fn from_handle(h: &CoreHandle) -> Ops {
        Ops {
            sessions: Arc::new(h.clone()),
            registry: Arc::new(h.clone()),
            history: Arc::new(h.clone()),
            discovery: Arc::new(h.clone()),
            events: h.events(),
        }
    }
}
