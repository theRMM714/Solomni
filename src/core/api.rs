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

/// 一次命令：在核心自己的线程上执行（因此核心状态不需要任何锁）。
type Job = Box<dyn FnOnce(&mut Core) + Send>;

/// 核心手柄：呈现层用它发命令。克隆廉价（内部只是通道 + 两个共享句柄）。
#[derive(Clone)]
pub struct CoreHandle {
    tx: Sender<Job>,
    jobs: Arc<JobRegistry>,
    bus: Arc<EventBus>,
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
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        // ① 短命令：检查 + 把会话取出来 + 定下本次调用参数（都在核心线程上，毫秒级）。
        let prepared = self.call({
            let sid = sid.to_string();
            let t = text.clone();
            move |core| core.prepare_single(&sid, t.as_deref(), out == Output::Stream)
        })?;
        let (session, prefix, llm) = match prepared {
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
                return self.queued_continue(sid, out);
            }
            Prepared::Run {
                session,
                prefix,
                llm,
            } => (*session, prefix, llm),
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
                    let mut events = prefix;
                    events.extend(match &text {
                        Some(t) => session.say(t, &mut live),
                        None => session.continue_reply(&mut live),
                    });
                    (session, events)
                })
                .map_err(|e| format!("起生成线程失败：{}", e))?
        };
        // ③ 收尾：拿回会话 → 入台 → 交回核心（重新插入 + 转录落盘）。
        //    **所有状态变更仍只发生在核心线程上**：工作线程只跑生成，不碰核心状态。
        let joined = worker.join();
        jobs.unregister(sid);
        let (session, events) = match joined {
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
        let seq = bus.push(sid, &events);
        self.call({
            let sid = sid.to_string();
            let ev = events.clone();
            move |core| {
                core.put_single(&sid, session, &ev);
                Ok(())
            }
        })?;
        Ok(Advance { events, seq })
    }

    /// 协作会话的"继续"：仍在核心线程上推进（B-1 只搬单 agent 生成，协作见后续）。
    fn queued_continue(&self, sid: &str, out: Output) -> Result<Advance, String> {
        let sid = sid.to_string();
        let jobs = Arc::clone(&self.jobs);
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let cancel = jobs.register(&sid);
            let mut emit = {
                let bus = Arc::clone(&bus);
                let sid = sid.clone();
                move |ev: SessionEvent| {
                    bus.push(&sid, std::slice::from_ref(&ev));
                }
            };
            let result = {
                let llm = core.llm_opts(out == Output::Stream);
                let mut live = Live {
                    llm,
                    cancel: Arc::clone(&cancel),
                    emit: &mut emit,
                };
                core.continue_flow(&sid, &mut live)
            };
            finish(&jobs, &bus, &sid, result)
        })
    }
}

/// 生成收尾：注销取消标志（无论成败）→ 本批事件入箱 → 返回事件与序号。
fn finish(
    jobs: &JobRegistry,
    bus: &EventBus,
    sid: &str,
    result: Result<Vec<SessionEvent>, String>,
) -> Result<Advance, String> {
    jobs.unregister(sid);
    let events = result?;
    let seq = bus.push(sid, &events);
    Ok(Advance { events, seq })
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
        let sid = sid.to_string();
        let text = text.to_string();
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let result = core.collab_continue(&sid, step, &text);
            let events = result?;
            let seq = bus.push(&sid, &events);
            Ok(Advance { events, seq })
        })
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
    ) -> Result<(), String> {
        let (id, name, api_model, provider, note) = (
            id.to_string(),
            name.to_string(),
            api_model.to_string(),
            provider.to_string(),
            note.to_string(),
        );
        self.call(move |core| core.model_upsert(&id, &name, &api_model, &provider, &note))
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
