//! 目的：运行时（第 10 层）的入站契约：呈现层与核心之间的唯一通道（见 docs/presentation/contracts.md）。
//! 管：`RuntimeOps`（启用 / 改需求 / 回档 / 停止）、动作词汇与 `ActionOps`、`LogOps`，以及呈现层共用读模型的再导出点。
//! 不管：建工作与视图（`work::api::WorkOps`）、回合推进（`turn::api::TurnOps`）、HTTP / JSON / 路由（呈现层的传输事）。
//! 联动：命令经 `Handle` 在核心自己的线程上执行；`entry/faces.rs` 把每个窄契约交给呈现层。

/// 目的：核心手柄（`service::handle::Handle`）：呈现层只经 `entry/faces.rs` 的 `Faces` 拆面用它发命令。
pub(crate) use crate::units::runtime::service::Handle;

/// 目的：文案目录与语言协商：呈现层按设置里的语言取词表，按稳定码取文本（不在界面复制词表）。
pub use crate::kernel::api::i18n;
/// 目的：回执说明的稳定码、类型化参数与轻重：呈现层按它上色与渲染，不解析回执正文。
pub use crate::kernel::api::note::Note;
pub use crate::kernel::api::note::Severity;
/// 目的：落盘历史的只读视图（会话列表是「在世会话 × 历史」的并集）。
#[allow(unused_imports)]
pub use crate::units::conversation::session::api::HistoryView;
/// 目的：回档目标与结果（层 9 的词汇）：这里只取用，不定义。
pub(crate) use crate::units::lifecycle::api::{RebuildDeps, RewindOutcome, RewindTarget};
/// 目的：工具通道探测的三态结论。
pub use crate::units::llm::api::ProbeOutcome;
/// 目的：运行档（本机档 / 虚拟机档）的词汇。
pub use crate::units::pure::exec_model::api::Tier;
/// 目的：会话事件（开场事实 / 回合事件 / 通知）的线格式。
pub use crate::units::transcript::api::SessionEvent;
/// 目的：裁决卡、裁决队与等待者：界面与 CLI 按它们渲染「等你定的事」。
#[allow(unused_imports)]
pub use crate::units::transcript::api::{DecisionCard, DecisionQueue, DecisionWaiter};
/// 目的：回合推进自用的协作态：门面重导出，契约测试的既有路径不必改。
pub(crate) use crate::units::turn::turn::api::CollabWork;
/// 目的：命令回包（事件台头部序号）、事件台与生成输出方式。
pub use crate::units::turn::turn::api::{Advance, Events, Output};
/// 目的：会话视图：启用后要交给界面的那一份（工作与会话配置的词汇归 `work::api`）。
pub use crate::units::work::api::SessionView;

/// 目的：运行时自己的用例：启用、改需求、回档、停止——它们要同时碰会话中心、历史生命周期与宿主的作业表，
///   只有 runtime 拿得到，所以归这里；身体只有编排，不含任何下层用例的实现。
pub trait RuntimeOps: Send + Sync {
    /// 目的：启用一条历史会话：从盘上重建、确保 guest、解冻整棵子树；**不产生模型调用**。
    fn activate(&self, sid: &str) -> Result<SessionView, String>;
    /// 目的：改需求：同样返回完整重放。
    fn update_task(&self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String>;
    /// 目的：回档：返回重放后的完整事件流（已是线格式，供前端整体重建）。
    /// 生产入口（动作表 / CLI）走 `Handle::rewind_with_reset`；这一格保留给
    /// 契约测试与其它调用方，同时确保回档的同步收尾只有一条实现。
    #[allow(dead_code)]
    fn rewind(&self, sid: &str, target: RewindTarget) -> Result<Vec<serde_json::Value>, String>;
    /// 目的：中断——取消整棵子树里正在跑的生成、作废等裁决的队；**不动运行态**（派发与唤醒照常）。
    /// 返回：**实际停下的会话**（空 = 本来就没在跑）。
    fn interrupt(&self, sid: &str) -> Vec<String>;
    /// 目的：停止会话——先把整棵子树落成 `stopped`（拦住后续派发与唤醒），再中断正在跑的生成。
    /// 返回：**实际停下的会话**；「继续」（`TurnOps::continue_flow`）是它的逆操作。
    fn stop(&self, sid: &str) -> Vec<String>;
}

/// 目的：日志能力：呈现层与 CLI 的埋点入口（**只转发，不做任何判定**）。
///   呈现层因此拿不到端口对象、也不依赖 kernel（见 docs/architecture/layers.md）。
pub trait LogOps: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    // 入口契约发布给前端的三个级别；warn 暂无调用点（呈现层的降级提示走它），先留着这一格。
    #[allow(dead_code)]
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

// ---------- 动作（声明在 systools/tools.yaml；分发归 runtime） ----------
/// 目的：一次动作的**调用者身份**——授权判据（动作表 `callers`）的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// 目的：人经呈现层（CLI / Web）调用。
    User,
    /// 目的：会话里的某个身份（模型经工具调用发起）：席位 id + 它所在的工作与会话。
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
/// 约束：这里只是**同一份声明**的投影——表单的缺省值、上下界、枚举与位置序号都从声明来。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionParamView {
    pub name: String,
    pub ty: String,
    pub required: bool,
    pub desc: String,
    /// 目的：缺省值（表单留空时用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// 目的：数值下界 / 上界（含）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// 目的：可选值清单（枚举）：前端该渲染成下拉而不是自由输入。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// 目的：位置参数序号（1 起）：没有就用具名输入。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positional: Option<u32>,
}

impl ActionParamView {
    /// 目的：从参数声明投影出一份呈现形态（两个目录入口共用，避免字段各写一遍）。
    pub fn of(name: &str, p: &crate::units::catalog::modules::api::Param) -> Self {
        ActionParamView {
            name: name.to_string(),
            ty: p.ty.name().to_string(),
            required: p.required,
            desc: p.desc.clone(),
            default: p.default.clone(),
            min: p.min,
            max: p.max,
            values: p.values.clone(),
            positional: p.positional,
        }
    }
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
