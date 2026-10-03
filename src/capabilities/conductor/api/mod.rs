//! 入站契约：呈现层（CLI / Web）与核心之间的唯一通道（见 docs/presentation/contracts.md）。
//!
//! 形态：**命令 + 事件**。
//! - 核心常驻自己的执行线程、独占全部状态：呈现层拿不到 `Conductor`，也拿不到任何核心锁。
//! - 呈现层只持有 `Ops`：四个**按角色切分**的能力接口 + 一个可订阅的事件台。
//! - 每条命令一次同步回复（std mpsc）：呈现层仍是「调用即拿结果」，不必改写成异步。
//! - **并发归核心**：停止/取消由 `JobRegistry` 承担，不进命令队列——所以生成期间照样立刻生效；
//!   呈现层因此不需要知道「生成时核心状态被占用」这类内部事实。
//! - 事件台由核心独占生产：任何数量的消费者按 `since` 增量取，序号供客户端去重。
//!
//! 这里**不出现 HTTP / JSON 封装 / 路由**：那些是呈现层的传输事（见 presentation/routes.rs）。
//! 事件与读模型（`SessionEvent`、`*View`）是**事实**的线格式，仍归 conductor。

// 登记处的入站契约归登记处自己：这里只用它的面（实现队列代理），不定义。
use crate::capabilities::conductor::service::Conductor;
use crate::capabilities::registry::api::AgentView;
use crate::capabilities::session::api::AgentMeta;
use crate::capabilities::session::api::HistoryView;
pub use crate::capabilities::session::api::{Pending, SessionEvent};
use crate::kernel::api::JobRegistry;
use crate::kernel::api::SessionId;
pub use crate::kernel::api::Tier;
use std::collections::BTreeMap;

// 入站契约返回的词汇：能力接口的返回类型在这里有一份**正式名字**。
// 呈现层只认这里，不直接碰 ports / providers 的内部路径。
pub use crate::capabilities::llm::api::ProbeOutcome;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

// ---------- 输出方式与推进结果 ----------

/// 一次生成的输出方式：流式逐片，还是只要最终结果。
/// 「怎么呈现」是呈现层的决定，所以由调用方显式给出，核心不替它猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Stream,
    Final,
}

/// 一次会话推进的结果：**只回事件台的头部序号**。
/// 命令不携带事实——事实只有一条来路（事件台）：想看就按 `since` 订阅，
/// 谁发起的命令都一样（CLI / Web / 桌面 / 演示脚本）。
#[derive(Debug, Clone)]
pub struct Advance {
    pub head: u64,
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

    /// 事件台当前头部序号：命令回包只给这个，调用方拿它当**订阅起点**。
    pub fn head(&self) -> u64 {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.seq
    }

    /// 测试专用：往事件台放一条空事件批，让长轮询立刻返回（不真等 20 秒）。
    #[cfg(test)]
    pub(crate) fn seed_for_test(&self, sid: &str) -> u64 {
        self.push(sid, &[])
    }

    /// 取 `since` 之后的事件批 + 当前头部 + **最老还留着的序号**（同一把锁内）。
    /// 为什么要把 oldest 给客户端：事件台会裁剪（BUS_MAX/BUS_KEEP），`since` 之后那一小段可能
    /// 已经永久没了。客户端据此**重新对齐**（拉一次历史重放），而不是按 seq 干等——干等的结果
    /// 是后续批次全部滞留，只有刷新页面才恢复（真机上就是这个症状：必须手动刷新才同步）。
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

    /// 盘上转录**之外**的实时尾巴 + 事件台当前头部：给「历史和实时一次给全」用（见呈现层的 history.open）。
    ///
    /// 只取「**转录在事件台里的最后一个位置之后**」的事实：转录是事件台的**前缀**，
    /// 它之前/之中的短暂事件（流式增量、开跑那条运行态）已经被盘上的定稿行取代了；
    /// 把它们也塞进尾巴，前端会先画定稿行、再画一个永远填不上的空"谁正在说"块。
    /// 判据是**结构化相等**（两边 JSON 出自同一套序列化器，就是同一个值），不是按行 id 猜：
    /// `notice`/`node_started` 这类行本来就没有 id，按 id 去重正是刷新后整段重复的来源。
    /// 返回的尾巴按事件台顺序展开，前端因此**只按序 append**，不自己合并两个来源。
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
        let (lines, head, _oldest) = self.snapshot(Some(sid), 0);
        let mut on_bus: Vec<serde_json::Value> = lines
            .into_iter()
            .flat_map(|l| l.events.into_iter().map(|e| e.to_json()))
            .collect();
        let mut start = 0usize;
        for (i, v) in on_bus.iter().enumerate() {
            if let Ok(k) = serde_json::to_string(v) {
                if let Some(n) = left.get_mut(&k) {
                    if *n > 0 {
                        *n -= 1;
                        start = i + 1;
                    }
                }
            }
        }
        let mut tail = Vec::new();
        for v in on_bus.drain(..).skip(start) {
            if let Ok(k) = serde_json::to_string(&v) {
                if let Some(n) = left.get_mut(&k) {
                    if *n > 0 {
                        *n -= 1;
                        continue;
                    }
                }
            }
            tail.push(v);
        }
        (tail, head)
    }
}

// ---------- 按角色切分的能力接口 ----------

/// 会话能力：创建、推进、回档、编辑、停止。会话界面只需要这一个。
pub trait SessionOps: Send + Sync {
    /// 建工作：回包是（会话与名单）+ **事件台头部**——开场事实只进事件台（命令不携带事实）。
    fn create_work(&self, spec: WorkSpec) -> Result<(WorkOpened, u64), String>;
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
    // 入站契约是**发布给前端的接口面**：二进制 crate 里暂时没有调用点的接口方法会被 dead_code 误报
    // （见 docs/testing/quality-isolation.md 的 allow 清单）。
    #[allow(dead_code)]
    fn exists(&self, sid: &str) -> Result<bool, String>;
    /// 工作名的缺省与唯一化（命名策略归 `session`）：`base` 去空白、为空用 `fallback`、重名加尾号。
    fn unique_work_name(&self, base: &str, fallback: &str) -> Result<String, String>;
    /// 请求停止该会话在跑的生成；返回是否确实有一个在跑。
    fn stop(&self, sid: &str) -> bool;
    #[allow(dead_code)]
    fn is_running(&self, sid: &str) -> bool;
    /// **在世会话 × 历史的并集**（界面上的会话列表）：只有会话中心同时知道两边，所以归这里。
    /// 落盘历史的列表 / 打开 / 删除归 `session::api::HistoryOps`。
    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String>;

    /// **动作分发**：一次动作 → 一次能力调用。CLI 与 Web 共用这一份（新增动作只改这里）。
    fn act(&self, sid: &str, action: Action<'_>, out: Output) -> Result<Acted, String> {
        match action {
            Action::Say(text) => self.say(sid, text, out).map(Acted::Advanced),
            Action::Continue => self.continue_flow(sid, out).map(Acted::Advanced),
            Action::Step(step, text) => self.collab_step(sid, step, text).map(Acted::Advanced),
            Action::Withdraw(agent) => self.withdraw_agree(sid, agent).map(Acted::Advanced),
            Action::Rewind(keep_id) => self.rewind(sid, keep_id).map(Acted::Replayed),
            Action::UpdateTask(text) => self.update_task(sid, text).map(Acted::Replayed),
            Action::Compact => self.compact(sid).map(Acted::Advanced),
        }
    }
}

/// 核心自己的用例（会话中心之外的那些）：运行报告与核心推荐。
///
/// 它们留在 conductor 是因为**只有 conductor 同时拿着**清单、包库、宿主探测与各能力的面——它们是编排
/// （§2.4 的脚本），没有独立状态，所以不配一个"能力"。清单事实本身归 `workspace::api::WorkspaceOps`。
pub trait ConductorOps: Send + Sync {
    /// 运行包与档位的运行报告（只报事实）。
    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String>;
    /// 新建工作时的**档位选择**（默认档 + 虚拟机档可用性与逐项前置）。
    /// 与「开始」的校验同源（同一把 `tier_readiness` 尺子），界面照抄，不自己编话。
    fn tier_choices(&self) -> Result<TierChoices, String>;
    /// 核心按任务推荐的 agent 草案（带理由；用户可改）。
    fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String>;
}

/// 日志能力：呈现层与 CLI 的埋点入口（**只转发，不做任何判定**）。
/// 呈现层因此拿不到端口对象、也不依赖 kernel（见 ARCHITECTURE.md §一）。
pub trait LogOps: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

// ---------- 核心手柄（命令通道） ----------

/// 协作在工作线程上要做的事：推进一个阶段，或从断点继续。
/// 为什么要分开：`CollabStep` 是**对外**的阶段枚举（前端按 pending 决定），
/// "继续"不是它的阶段之一（前端走 `continue_flow`），所以内部再分一层，不污染对外契约。
#[derive(Clone, Copy)]
pub(crate) enum CollabWork {
    Step(CollabStep),
    Resume,
}

/// 一次命令：在核心自己的线程上执行（因此核心状态不需要任何锁）。
type Job = Box<dyn FnOnce(&mut Conductor) + Send>;

/// 核心手柄：呈现层用它发命令。克隆廉价（内部只是通道 + 两个共享句柄）。
#[derive(Clone)]
pub struct ConductorHandle {
    tx: Sender<Job>,
    jobs: Arc<JobRegistry>,
    bus: Arc<EventBus>,
    /// 日志句柄：呈现层经 LogOps 能力写日志，拿不到这个端口对象本身。
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 代理会话注入成员侧执行者要的两份装配材料（与核心共享同一份，不在桥这一侧重装）。
    book: crate::capabilities::tools::api::ToolBook,
    texts: Arc<crate::capabilities::prompt::api::ToolTexts>,
}

/// 一次"要一个成员回合"的请求：泵在工作线程上让出，回头找主线程驱动（它才拿得到各 agent 的会话）。
pub(crate) struct AskReq {
    agent: String,
    /// 本回合的**身份块**（由泵按当前提示词册现渲染）。
    identity: String,
    /// 本回合的提示（开场词 / 轮转词）。
    turn: Vec<crate::capabilities::llm::api::Msg>,
    /// 工具总表与角色表的能力面（**共享一份**）：发放工具面与校验越权都用它。
    systools: std::sync::Arc<dyn crate::capabilities::tools::api::Tools>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    opts: crate::capabilities::llm::api::CompleteOpts<'static>,
    /// 这一回合属于第几轮（写进 agent 会话的回合标记）。
    round: usize,
    /// 回合 id（整场工作单调递增；两边对得上就靠它）。
    turn_id: u64,
}
mod handle;
mod proxy;

/// 入站能力面：**各能力自己的契约**+ 核心自己的两个 + 事件台。
/// 克隆廉价；呈现层只依赖它需要的字段，契约测试可以用假实现替换任意一个字段。
#[derive(Clone)]
pub struct Ops {
    /// 会话中心（conductor 自己的状态）：会话生命周期、动作分发、文件视图、会话总览。
    pub sessions: Arc<dyn SessionOps + Send + Sync>,
    /// 核心自己的用例：运行报告与核心推荐。
    pub core: Arc<dyn ConductorOps + Send + Sync>,
    pub registry: Arc<dyn crate::capabilities::registry::api::RegistryOps + Send + Sync>,
    pub history: Arc<dyn crate::capabilities::session::api::HistoryOps + Send + Sync>,
    pub workspace: Arc<dyn crate::capabilities::workspace::api::WorkspaceOps + Send + Sync>,
    pub events: Arc<EventBus>,
    /// 日志能力：呈现层只经它埋点（**不持有端口对象**）。
    pub log: Arc<dyn LogOps + Send + Sync>,
}

impl Ops {
    /// 组合根用：把同一个核心手柄按角色拆开（同一个执行线程、同一个事件台）。
    pub fn from_handle(h: &ConductorHandle) -> Ops {
        Ops {
            sessions: Arc::new(h.clone()),
            core: Arc::new(h.clone()),
            registry: Arc::new(h.clone()),
            history: Arc::new(h.clone()),
            workspace: Arc::new(h.clone()),
            events: h.events(),
            log: Arc::new(h.clone()),
        }
    }
}

// ---------- 入站词汇（呈现层与核心共用的形状） ----------

/// 一次会话动作（**用例词汇**）：CLI 与 Web 共用同一分发（新增动作只改这里）。
#[derive(Debug, Clone, Copy)]
pub enum Action<'a> {
    /// 单 agent 说一句。
    Say(&'a str),
    /// 继续一次会话。
    Continue,
    /// 协作推进到下一阶段。
    Step(CollabStep, &'a str),
    /// 撤回同意。
    Withdraw(&'a str),
    /// 回档到某行之前。
    Rewind(u64),
    /// 改需求。
    UpdateTask(&'a str),
    /// 压缩上下文（AI 自己压成摘要；此后此前内容不再发给模型，用户仍可查看）。
    Compact,
}

/// 动作结果：生成类只回**事件台头部序号**（事实在事件台上，订阅者自己按 since 取）；
/// 回档/改需求给完整重放（那是快照，不是增量事实）。
pub enum Acted {
    Advanced(Advance),
    Replayed(Vec<serde_json::Value>),
}

//
// 定义在**能力面**：呈现层只认这里，不再经 `conductor::` 根转一手。
// 队列代理：只把命令交给核心线程（依赖方向见 ARCHITECTURE.md §一）。
/// 协作推进阶段：由前端按 pending 决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollabStep {
    SetTask,
    ConfirmSlate,
    Begin,
    /// **用户对裁决的自由文本回应**：核心 AI 判定意图是否明确，明确了才开工/放行。
    Decide,
}

/// 工作形态：单 agent（模块数不限）/ 协作（多 agent 分权协商）/ 代理（决定权整块交给核心）。
/// 形态只用于校验与界面标签：会话实现只有「单 agent」与「协作」两种——
/// 代理会话在实现上就是一个单会话（`mode="proxy"`），只是身份换成 `core_proxy` 角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkMode {
    Single,
    Collab,
    /// 代理：**没有名单**；用户选这一形态就是在授予全权（见 `SessionMeta::delegation`）。
    Proxy,
}

/// 一次工作里的一个 agent 实例（用户选定，或核心代拟的临时组合）。
/// agent = 一个 AI 实例 + N 份模块能力 + 一个模型 + 一个沙箱。
#[derive(Debug, Clone)]
pub struct AgentInstance {
    pub name: String,
    /// 是否临时 agent（不来自 .home/agents.yaml，只随本次工作落档）。
    pub transient: bool,
    pub modules: Vec<String>,
    /// 该 agent 的模型（模型 id）；None = 核心默认。
    pub model: Option<String>,
}

impl AgentInstance {
    /// 登记处视图 → 用例输入。**唯一**的转换口径（点名与「无参 = 全部」两条路共用）。
    pub fn from_view(v: &AgentView) -> AgentInstance {
        AgentInstance {
            name: v.name.clone(),
            transient: false,
            modules: v.modules.clone(),
            model: v.model.clone(),
        }
    }
}

/// 创建工作的全部用户决定（形态 + 参与的 agent；模型按 agent 指定）。
#[derive(Debug, Clone)]
pub struct WorkSpec {
    pub name: String,
    pub mode: WorkMode,
    pub agents: Vec<AgentInstance>,
    /// 协作：本次需求（必填）。
    pub task: Option<String>,
    /// 保留：委托核心代拟名单（协作且未给 agent 时）。
    pub delegate: bool,
    /// 执行档位（用户在创建向导里选的；默认 = 设置里的档位）。承载不了就由 create_work 如实拒绝。
    pub tier: Tier,
}

/// 创建工作后的结果：最终实例名（重名已加尾号）、名单，以及**开场事实**。
/// 事实由 api 层发布进事件台（核心不持有事件台）；**不上回包**——
/// 回包只给事件台头部序号，谁要看谁按 `since` 订阅。
#[derive(Debug, Clone)]
pub struct WorkOpened {
    pub sid: SessionId,
    pub agents: Vec<String>,
    pub facts: Vec<SessionEvent>,
}

/// 核心推荐的 agent 草案（名字 + 模块 + 模型 + 理由；用户可改，核心不代选）。
/// reuse = 直接复用登记处已存的 agent（模块与模型都取它自己的，核心不代拟模型）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentSuggestion {
    pub name: String,
    pub modules: Vec<String>,
    pub model: String,
    pub why: String,
    pub reuse: bool,
}

/// 运行能力报告（呈现与日志用）：声明了什么、包库里有什么、缺什么、虚拟机档的诊断。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeReport {
    pub tier: String,
    /// 模块 id → 它声明的运行能力（升序）。
    pub declared: BTreeMap<String, Vec<String>>,
    /// 能力名 → 包库里的可用版本（升序）。
    pub available: BTreeMap<String, Vec<String>>,
    /// 模块 id → 包库里没有的能力（档位无关的事实）。
    pub missing: BTreeMap<String, Vec<String>>,
    /// 虚拟机档下不能成立的诊断；本机档为空。
    pub diagnoses: Vec<crate::capabilities::workspace::api::Diagnosis>,
    /// 本机能不能承载**当前档位**（虚拟机档的前置条件；本机档恒为可）。
    /// 界面据此决定虚拟机档能不能选，并与「开始」/编辑的校验同源（`crate::capabilities::workspace::api::tier_readiness`）。
    pub tier_ready: bool,
    /// 承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// 被拒收的模块（原因如实）。
    pub rejected: Vec<String>,
    /// 被拒收的运行包（原因如实）。
    pub rejected_packages: Vec<String>,
}

/// 新建工作时的档位选择视图（创建向导用）：默认档 + 虚拟机档为什么不能选（逐项前置）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TierChoices {
    /// 默认档位（设置里的 `tier`）。
    pub default: String,
    /// **虚拟机档**能不能选（与当前档位无关）。
    pub vm_available: bool,
    /// 不能选时的理由（能选 = 空）。
    pub vm_unavailable_reason: String,
    /// 虚拟机档的**逐项前置**（缺哪几项、每项怎么补）：界面照抄，不自己编话。
    pub vm_requirements: Vec<crate::capabilities::workspace::api::VmRequirement>,
}

/// 配置界面里的一个 agent（名字冻结时仍要显示）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfigAgent {
    pub name: String,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub model: String,
}

/// 会话配置视图（配置界面用）：身份与冻结标记 + 可改项 + 运行能力事实。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionConfig {
    pub sid: String,
    pub mode: String,
    /// 会话已经开过（流水里有内容）= agent 名单与形态冻结。
    pub started: bool,
    pub agents: Vec<ConfigAgent>,
    pub tier: String,
    pub base: Option<String>,
    pub net: bool,
    pub pins: BTreeMap<String, String>,
    /// 本机能不能承载**当前档位**（虚拟机档的前置条件）：界面据此决定虚拟机档能不能选。
    pub tier_ready: bool,
    /// 承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// **虚拟机档**能不能选（与当前档位无关）：为假时界面禁用虚拟机档，编辑与「开始」也会拒绝。
    pub vm_available: bool,
    /// 虚拟机档为什么不能选（`vm_available` 为真时为空）。
    pub vm_unavailable_reason: String,
    /// 虚拟机档的**逐项前置**（缺哪几项、每项怎么补）：界面照抄，不自己编话。
    pub vm_requirements: Vec<crate::capabilities::workspace::api::VmRequirement>,
    /// 运行能力报告（模块声明 / 包库可用 / 缺包 / 虚拟机档诊断 / 拒收原因）。
    pub runtime: RuntimeReport,
    /// 依赖文件夹（把运行包放进这里；真实路径，给用户看）。
    pub runtimes_dir: String,
}

/// 编辑提交（配置界面用）：改模块、模型、档位、定版与网络。
/// 名字与形态不在这里——会话一旦开过（流水有内容）它们就冻结了。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SessionEdit {
    pub agents: Vec<ConfigAgent>,
    pub tier: String,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    #[serde(default)]
    pub net: bool,
}

/// 会话列表视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionView {
    pub sid: String,
    pub mode: String,
    pub done: bool,
    /// 这条会话记的执行档位（`host` / `vm`）：打开时据此提示"环境已变"。
    pub tier: String,
    /// 记的档位**现在还能不能承载**（本机档恒真）。为假时前端打开前给提示，但不拦打开。
    pub tier_ready: bool,
    /// 承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// **这条会话此刻在跑吗**（生成中）：界面据此把「发送/继续」换成「停止」并显示占位动画。
    /// 它是**对账副本**：运行态的唯一真相是推的 `SessionEvent::Working`（短暂、不落盘）；
    /// 这份快照供界面在"本页对这条会话还没有实时知识"时对齐——刚刷新页面、事件台裁掉一段后
    /// 重连、别人建的会话（见 docs/session/session-model.md 二之二）。
    pub running: bool,
    /// **这条工作有「本次需求」吗**：前端据此决定要不要渲染「改需求」按钮——
    /// 没有就**根本不渲染**（不是灰着）。这是领域事实（有没有需求行），不是"模式"。
    pub can_update_task: bool,
    /// 运行态（`active` / `paused` / `closed`）：**持久事实**，与短暂的 `running` 分开。
    /// 界面据此标出"已暂停 / 已关闭"（这类会话不会再被派发或唤醒）。
    pub run: String,
    /// 当前等用户裁决的事（None = 没有）：**快照形态**，与推的 `SessionEvent::Decision` 同源。
    /// 刷新页面时界面照样画得出那张卡；推的那条只是增量。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<serde_json::Value>,
}

/// 会话文件清单视图（前端 @ 菜单与「长路径缩写」用）：相对清单 + 真实根。
/// 根一律是 slash() 书写形式（/ 分隔、无扩展长度前缀）；拿不到根就给空串（前端原样显示，不猜）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilesView {
    pub work: Vec<String>,
    pub agents: Vec<FilesAgentView>,
    pub roots: FilesRootsView,
    /// 工作区用量（文件数与总字节）：删除前如实交代"还有多少东西会被一起删"。
    pub usage: crate::capabilities::workspace::api::WorkUsage,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilesAgentView {
    pub name: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilesRootsView {
    pub work: String,
    pub agents: Vec<FilesAgentRootView>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FilesAgentRootView {
    pub name: String,
    pub root: String,
}
