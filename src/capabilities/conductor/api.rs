//! 目的：呈现层（CLI / Web）与核心之间的唯一通道（见 docs/presentation/contracts.md）：命令 + 事件。
//! 管：`Ops` 的四个按角色切分的能力接口，以及两端共用的入站读模型（`WorkSpec`、`SessionEdit`、`RuntimeReport`、`*View` 等）。
//! 不管：HTTP / JSON 封装 / 路由（呈现层的传输事，见 presentation/routes.rs）；回合推进与事件台归 turn；核心状态与并发归 service。
//! 联动：命令经 `ConductorHandle` 在核心自己的线程上执行，回包只带事件台头部序号（`turn::api::Advance`）。

// 登记处的入站契约归登记处自己：这里只用它的面（实现队列代理），不定义。
use crate::capabilities::conductor::service::Conductor;
use crate::capabilities::session::api::HistoryView;
pub use crate::capabilities::session::api::SessionEvent;
pub use crate::capabilities::session::api::{DecisionCard, DecisionQueue, DecisionWaiter};
use crate::capabilities::work::api::{
    AgentSuggestion, FilesView, RuntimeReport, SessionConfig, SessionEdit, SessionView,
    TierChoices, WorkMode, WorkOpened, WorkSpec,
};
// 回档目标与结果是层 10 的词汇：这里只取用（子模块经 `use super::*` 一并拿到），不定义、不转发。
pub(crate) use crate::capabilities::lifecycle::api::{RebuildDeps, RewindOutcome, RewindTarget};
use crate::kernel::api::JobRegistry;
pub use crate::kernel::api::Tier;

// 入站契约返回的词汇：能力接口的返回类型在这里有一份**正式名字**。
// 呈现层只认这里，不直接碰 ports / providers 的内部路径。
pub use crate::capabilities::llm::api::ProbeOutcome;
use std::sync::Arc;

// 回合推进自用的值类型与事件台归 turn：这里只做门面重导出，呈现层与测试的既有路径不必改。
pub(crate) use crate::capabilities::turn::api::CollabWork;
pub use crate::capabilities::turn::api::{Advance, EventBus, Output};

/// 目的：会话能力：创建、推进、回档、编辑、停止。会话界面只需要这一个。
pub trait SessionOps: Send + Sync {
    /// 目的：建工作——回包是（会话与名单）+ **事件台头部**，开场事实只进事件台（命令不携带事实）。
    /// 约束：呈现侧建会话统一走动作表（`create_session`）；这一格保留给契约测试与其它调用方，
    /// 二进制 crate 里没有调用点会被 dead_code 误报（见 docs/testing/quality-isolation.md 的 allow 清单）。
    #[allow(dead_code)]
    fn create_work(&self, spec: WorkSpec) -> Result<(WorkOpened, u64), String>;
    /// 目的：单 agent 会话里说一句（生成可被 `stop` 中止）。
    fn say(&self, sid: &str, text: &str, out: Output) -> Result<Advance, String>;
    /// 目的：启用一条历史会话：从盘上重建、确保 guest、解冻整棵子树；**不产生模型调用**。
    fn activate(&self, sid: &str) -> Result<SessionView, String>;
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
    /// 目的：回档：返回重放后的完整事件流（已是线格式，供前端整体重建）。
    /// 生产入口（动作表 / CLI）走 `ConductorHandle::rewind_with_reset`；这一格保留给
    /// 契约测试与其它调用方，同时确保回档的同步收尾只有一条实现。
    #[allow(dead_code)]
    fn rewind(&self, sid: &str, target: RewindTarget) -> Result<Vec<serde_json::Value>, String>;
    /// 目的：压缩这个会话的上下文（AI 自己压；压不动如实说）。
    fn compact(&self, sid: &str) -> Result<Advance, String>;
    /// 目的：改需求：同样返回完整重放。
    fn update_task(&self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String>;
    fn config(&self, sid: &str) -> Result<SessionConfig, String>;
    fn edit(&self, sid: &str, edit: SessionEdit) -> Result<(), String>;
    /// 目的：投喂文件进本次工作的 work/；返回 false = 同名已存在（由用户决定覆盖或改名）。
    fn upload(&self, sid: &str, name: &str, bytes: &[u8], overwrite: bool) -> Result<bool, String>;
    fn files(&self, sid: &str) -> Result<FilesView, String>;
    /// 目的：提交历史（只读）：当前 head + 分页提交；`before` = 只取 id 更小的提交。
    fn work_log(
        &self,
        sid: &str,
        limit: usize,
        before: Option<u64>,
    ) -> Result<crate::capabilities::workspace::api::WorkLog, String>;
    /// 目的：一条提交的详情（只读）。
    fn work_commit_view(
        &self,
        sid: &str,
        commit: u64,
    ) -> Result<crate::capabilities::workspace::api::CommitDetail, String>;
    /// 目的：一条提交里某个文件的内容预览（只读；路径必须在这条提交的树里）。
    fn work_commit_file(
        &self,
        sid: &str,
        commit: u64,
        path: &str,
    ) -> Result<crate::capabilities::workspace::api::CommitFileView, String>;
    // 入站契约是**发布给前端的接口面**：二进制 crate 里暂时没有调用点的接口方法会被 dead_code 误报
    // （见 docs/testing/quality-isolation.md 的 allow 清单）。
    #[allow(dead_code)]
    fn exists(&self, sid: &str) -> Result<bool, String>;
    /// 目的：工作名的缺省与唯一化（命名策略归 `session`）：`base` 去空白、为空用 `fallback`、重名加尾号。
    fn unique_work_name(&self, base: &str, fallback: &str) -> Result<String, String>;
    /// 目的：停止——把整棵子树落成 `stopped`（拦住后续派发与唤醒）并中断正在跑的生成。
    /// 返回：**实际停下的会话**（空 = 本来就没在跑）；「继续」（`continue_flow`）是它的逆操作。
    fn stop(&self, sid: &str) -> Vec<String>;
    #[allow(dead_code)]
    fn is_running(&self, sid: &str) -> bool;
    /// 目的：**在世会话 × 历史的并集**（界面上的会话列表）：只有会话中心同时知道两边，所以归这里。
    /// 落盘历史的列表 / 打开 / 删除归 `session::api::HistoryOps`。
    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String>;
}

/// 目的：核心自己的用例（会话中心之外的那些）：运行报告与核心推荐。
///   它们留在 conductor 是因为**只有 conductor 同时拿着**清单、包库、宿主探测与各能力的面——它们是编排
///   （§2.4 的脚本），没有独立状态，所以不配一个"能力"。清单事实本身归 `modules::api::ModulesOps`。
pub trait ConductorOps: Send + Sync {
    /// 目的：运行包与档位的运行报告（只报事实）。
    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String>;
    /// 目的：新建工作时的**档位选择**（默认档 + 虚拟机档可用性与逐项前置）。
    /// 与「开始」的校验同源（同一把 `tier_readiness` 尺子），界面照抄，不自己编话。
    fn tier_choices(&self) -> Result<TierChoices, String>;
    /// 目的：核心按任务推荐的 agent 草案（带理由；用户可改）。
    fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String>;
}

/// 目的：日志能力：呈现层与 CLI 的埋点入口（**只转发，不做任何判定**）。
///   呈现层因此拿不到端口对象、也不依赖 kernel（见 ARCHITECTURE.md §一）。
pub trait LogOps: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    // 入口契约发布给前端的三个级别；warn 暂无调用点（呈现层的降级提示走它），先留着这一格。
    #[allow(dead_code)]
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

/// 目的：核心手柄：呈现层用它发命令。克隆廉价（内部只是通道 + 两个共享句柄）。
#[derive(Clone)]
pub struct ConductorHandle {
    /// 目的：命令队列：核心状态只在它自己那条线程上被执行（机制壳在 `src/kernel/domain/actor.rs`）。
    actor: crate::kernel::api::Actor<Conductor>,
    jobs: Arc<JobRegistry>,
    /// 目的：有没有"能作答"的交互前端（Web 在服务时打开）：打开才把工具级确认接进裁决队。
    /// 纯终端同步生成不打开，避免生成线程空等一个没人回答的问题。
    interactive: Arc<std::sync::atomic::AtomicBool>,
    bus: Arc<EventBus>,
    /// 目的：日志句柄：呈现层经 LogOps 能力写日志，拿不到这个端口对象本身。
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 目的：代理会话注入成员侧执行者要的两份装配材料（与核心共享同一份，不在桥这一侧重装）。
    book: crate::capabilities::tools::api::ToolBook,
    /// 目的：工具执行面："人直接跑一个模块工具"（无会话）也走它，不另起一套执行机制。
    tools: Arc<dyn crate::capabilities::tools::api::ToolExec + Send + Sync>,
    /// 目的：常驻服务面（与核心共享同一份）：ResidentOps 由它转发。
    residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    /// 目的：隐秘字段面（与核心共享同一份）：SecretOps 由它转发。
    secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    /// 目的：模块公地能力面（与核心共享同一份）：模块事实与启停写面由它转发。
    modules: Arc<dyn crate::capabilities::modules::api::Modules + Send + Sync>,
    /// 目的：构建能力面（与核心共享同一份）：创建会话前在**调用方线程**补编模块产物。
    builds: Arc<dyn crate::capabilities::builds::api::Builds + Send + Sync>,
    /// 目的：正在启动的 guest 租约：同一台 guest 的并发启用/生成只能有一个在拉，别互相替换。
    booting: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}

mod action;
mod contract_impls;
mod handle;

/// 目的：入站能力面：**各能力自己的契约**+ 核心自己的两个 + 事件台。
///   克隆廉价；呈现层只依赖它需要的字段，契约测试可以用假实现替换任意一个字段。
#[derive(Clone)]
pub struct Ops {
    /// 目的：会话中心（conductor 自己的状态）：会话生命周期、动作分发、文件视图、会话总览。
    pub sessions: Arc<dyn SessionOps + Send + Sync>,
    /// 目的：核心自己的用例：运行报告与核心推荐。
    pub core: Arc<dyn ConductorOps + Send + Sync>,
    pub registry: Arc<dyn crate::capabilities::registry::api::RegistryOps + Send + Sync>,
    pub history: Arc<dyn crate::capabilities::session::api::HistoryOps + Send + Sync>,
    /// 目的：模块公地的只读面（清单事实的上游是 `modules`）。
    pub modules: Arc<dyn crate::capabilities::modules::api::ModulesOps + Send + Sync>,
    pub events: Arc<EventBus>,
    /// 目的：动作能力：目录 + 分发（CLI 与 Web 共用同一份声明与同一处授权）。
    pub actions: Arc<dyn ActionOps + Send + Sync>,
    /// 目的：日志能力：呈现层只经它埋点（**不持有端口对象**）。
    pub log: Arc<dyn LogOps + Send + Sync>,
    /// 目的：常驻服务的统一管理面（组合根注入的同一份，经核心手柄转发）。
    pub residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    /// 目的：隐秘字段的统一管理面（组合根注入的同一份，经核心手柄转发）。
    pub secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    /// 目的：依赖补齐的统一面（组合根注入；默认未接入，如实失败不假装可用）。
    pub provision: Arc<dyn crate::capabilities::provision::api::Provision + Send + Sync>,
    /// 目的：存储空间的统一面（组合根注入；默认空实现，测试可替换）。
    pub storage: Arc<dyn crate::capabilities::storage::api::Storage + Send + Sync>,
    /// 目的：基础工具链的启动事实（组合根注入；测试替身为 null）。
    pub toolchains: serde_json::Value,
}

impl Ops {
    /// 目的：组合根用：把同一个核心手柄按角色拆开（同一个执行线程、同一个事件台）。
    pub fn from_handle(h: &ConductorHandle) -> Ops {
        Ops {
            sessions: Arc::new(h.clone()),
            core: Arc::new(h.clone()),
            registry: Arc::new(h.clone()),
            history: Arc::new(h.clone()),
            modules: Arc::new(h.clone()),
            events: h.events(),
            actions: Arc::new(h.clone()),
            log: Arc::new(h.clone()),
            residents: Arc::clone(&h.residents),
            secrets: Arc::clone(&h.secrets),
            provision: Arc::new(crate::capabilities::provision::api::NoProvision),
            storage: Arc::new(crate::capabilities::storage::api::NoStorage),
            toolchains: serde_json::Value::Null,
        }
    }
}

// ---------- 动作（声明在 systools/tools.yaml；分发归 conductor） ----------
/// 目的：一次动作的**调用者身份**——授权判据（动作表 `callers`）的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// 目的：人经呈现层（CLI / Web）调用。
    User,
    /// 目的：会话里的某个身份（模型经工具调用发起）：角色 id + 它所在的工作与会话。
    Role {
        role: String,
        work: String,
        agent: String,
    },
}

impl Caller {
    /// 目的：动作表 `callers` 里代表它的身份串（授权比对只认它）。
    pub fn token(&self) -> &str {
        match self {
            Caller::User => "user",
            Caller::Role { role, .. } => role,
        }
    }
}

/// 目的：一次动作请求——动作 id + **已解析的参数对象** + 调用者身份 + 输出方式。
/// 约束：参数校验、授权、执行、审计都在 `ActionOps::act` 一处完成；两个适配器只负责造出它。
#[derive(Debug, Clone)]
pub struct ActionCall {
    pub id: String,
    pub args: serde_json::Value,
    pub caller: Caller,
    pub out: Output,
}

/// 目的：目录里一条参数的呈现形态（前端与 CLI 照它生成输入，不硬编码参数名）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionParamView {
    pub name: String,
    pub ty: String,
    pub required: bool,
    pub desc: String,
}

/// 目的：动作目录里的一条——**这个调用者此刻能做什么**。前端据此渲染，不写第二份动作清单。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionView {
    pub id: String,
    pub desc: String,
    pub params: Vec<ActionParamView>,
    /// 目的：这个调用者此刻能不能调（授权通过 + 此刻适用）。
    pub available: bool,
    /// 目的：不能调时的原因（能调时为空）。
    pub reason: String,
}

/// 目的：动作能力的入站面——目录 + 分发。
pub trait ActionOps: Send + Sync {
    /// 目的：目录：给这个调用者能看到的动作（含此刻可用性）。`sid` = 当前会话上下文。
    fn catalog(&self, caller: &Caller, sid: Option<&str>) -> Result<Vec<ActionView>, String>;
    /// 目的：分发一次动作：参数校验 → 按 `callers` 授权 → 执行 → 审计，各只有一处。
    fn act(&self, call: ActionCall) -> Result<Acted, String>;
}

/// 目的：动作结果：生成类只回**事件台头部序号**（事实在事件台上，订阅者自己按 since 取）；
///   回档 / 改需求给完整重放（那是快照，不是增量事实）；其余给一份结构化结果（如 `{ok:true}`）。
#[derive(Debug)]
pub enum Acted {
    Advanced(Advance),
    Replayed(Vec<serde_json::Value>),
    Done(serde_json::Value),
}
