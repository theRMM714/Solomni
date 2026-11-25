//! 目的：work 能力对外的契约面：一次工作的用户决定与结果（`WorkSpec` / `WorkOpened`）、
//!   会话与配置的读模型（`SessionView` / `SessionConfig` / `FilesView` / `RuntimeReport` 等），
//!   以及"要等 guest"的那几段自足任务（`GuestBoot` / `EditGuest` / `GuestDiagnose`；单次销毁 `GuestStop` 在 `vm`）。
//! 管：类型的定义与文档（值的唯一家）。
//! 不管：怎么建、怎么改、怎么报（在 service 里）；呈现层的 JSON 封装。
//! 联动：runtime 与呈现层都从这里取类型；guest 任务的 `run()` 在调用方线程上跑。

use crate::kernel::api::types::SessionId;
use crate::kernel::ports::{HostProbe, Log};
use crate::units::catalog::registry::api::AgentMeta;
use crate::units::conversation::session::api::HistoryView;
use crate::units::pure::exec_model::api::Tier;
use crate::units::transcript::api::{OptionView, SessionEvent};
use crate::units::vm::api::{ExecPlan, GuestStop, Vm};
use crate::units::workspace::api::Sandboxes;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：创建工作的全部用户决定（会话类型 + 参与的 agent；模型按 agent 指定）。
#[derive(Debug, Clone)]
pub struct WorkSpec {
    pub name: String,
    /// 目的：会话类型名（表里的预设，如 单机 / 会议 / 代理；创建时解析成 `SessionSpec` 落盘）；临时拼装时不用。
    pub kind: String,
    /// 目的：临时拼装的流程 id（Some = 不走预设，按这几个流程建；装配前过白名单校验）。
    pub flows: Option<Vec<String>>,
    pub agents: Vec<AgentMeta>,
    /// 目的：协作：本次需求（必填）。
    pub task: Option<String>,
    /// 目的：保留：委托核心代拟名单（协作且未给 agent 时）。
    pub delegate: bool,
    /// 目的：执行档位（用户在创建向导里选的；默认 = 设置里的档位）。承载不了就由 create_work 如实拒绝。
    pub tier: Tier,
    /// 目的：创建时选的权限档位；None = 用设置里的默认档（代理创建时由呈现层显式选）。
    pub permission: Option<crate::units::pure::permission::api::ProfileId>,
    /// 目的：虚拟机档用的虚拟根名（可省 = 空；会话建好后也可在会话配置里改）。
    pub guest: Option<String>,
    /// 目的：外部工作区根（已规范化的绝对路径）；None = 产品内 `session/<名>/work`。
    /// 约束：只有顶层会话能指定；子会话沿父链继承同一个根。
    pub workspace: Option<std::path::PathBuf>,
    /// 目的：创建时就打开过期内容替换（None = 关）：`(开, 保底百分比)`，保底 0..=90。
    /// 约束：只写在工作根 meta 上；开启且没给百分比时用默认 80。
    pub freshness: Option<(bool, u8)>,
}

/// 目的：创建工作后的结果：最终实例名（重名已加尾号）、名单，以及**开场事实**。
///   事实由 api 层发布进事件台（核心不持有事件台）；**不上回包**——
///   回包只给事件台头部序号，谁要看谁按 `since` 订阅。
#[derive(Debug, Clone)]
pub struct WorkOpened {
    pub sid: SessionId,
    pub agents: Vec<String>,
    pub facts: Vec<SessionEvent>,
}

/// 目的：核心推荐的 agent 草案（名字 + 模块 + 模型 + 理由；用户可改，核心不代选）。
///   reuse = 直接复用登记处已存的 agent（模块与模型都取它自己的，核心不代拟模型）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentSuggestion {
    pub name: String,
    pub modules: Vec<String>,
    pub model: String,
    pub why: String,
    pub reuse: bool,
    /// 目的：人设 / 身份（复用项取登记处自己的；组装项取核心建议）。
    pub chara: String,
}

/// 目的：运行能力诊断的**组合**：包选择问题与模块入口问题各归其所有者，报告只做聚合。
/// 约束：`untagged` 让前端拿到的 JSON 仍是扁平的 `{ "Missing": ... }` / `{ "MissingEntry": ... }`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum RuntimeDiagnosis {
    Package(crate::units::catalog::packages::api::Problem),
}

/// 目的：组合诊断的可读说法（CLI / 日志用）。
pub fn diagnose_text(diags: &[RuntimeDiagnosis]) -> String {
    diags
        .iter()
        .map(|d| match d {
            RuntimeDiagnosis::Package(p) => {
                crate::units::catalog::packages::api::diagnose_text(std::slice::from_ref(p))
            }
        })
        .collect::<Vec<_>>()
        .join("；")
}

/// 目的：运行能力报告（呈现与日志用）：声明了什么、包库里有什么、缺什么、档位诊断。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeReport {
    pub tier: String,
    /// 目的：模块 id → 它声明的运行能力（升序）。
    pub declared: BTreeMap<String, Vec<String>>,
    /// 目的：能力名 → 包库里的可用版本（升序）。
    pub available: BTreeMap<String, Vec<String>>,
    /// 目的：模块 id → 本档位下用不了的能力。
    pub missing: BTreeMap<String, Vec<String>>,
    /// 目的：本档位下不能成立的诊断（包选择 + 模块入口）；空 = 没有。
    pub diagnoses: Vec<RuntimeDiagnosis>,
    /// 目的：本机能不能承载**当前档位**（虚拟机档的前置条件；本机档恒为可）。
    ///   界面据此决定虚拟机档能不能选，并与「开始」/编辑的校验同源（`crate::units::vm::api::tier_readiness`）。
    pub tier_ready: bool,
    /// 目的：承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// 目的：被拒收的模块（原因如实）。
    pub rejected: Vec<String>,
    /// 目的：被拒收的运行包（原因如实）。
    pub rejected_packages: Vec<String>,
}

/// 目的：新建工作时的档位选择视图（创建向导用）：默认档 + 虚拟机档为什么不能选（逐项前置）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TierChoices {
    /// 目的：默认档位（设置里的 `tier`）。
    pub default: String,
    /// 目的：**虚拟机档**能不能选（与当前档位无关）。
    pub vm_available: bool,
    /// 目的：不能选时的理由（能选 = 空）。
    pub vm_unavailable_reason: String,
    /// 目的：虚拟机档的**逐项前置**（缺哪几项、每项怎么补）：界面照抄，不自己编话。
    pub vm_requirements: Vec<crate::units::vm::api::VmRequirement>,
    /// 目的：本机可选的**虚拟根**（虚拟机档那一步的下拉项）——界面照抄，不自己编列表。
    pub guests: Vec<crate::units::provision::api::GuestBase>,
    /// 目的：包库里可用的能力名（→ 版本）：界面据此预判"这个虚拟根 + 包库满足不了哪些模块"。
    pub available: BTreeMap<String, Vec<String>>,
}

/// 目的：配置界面里的一个 agent（名字冻结时仍要显示）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigAgent {
    pub name: String,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub model: String,
    /// 目的：人设 / 身份（会话快照；配置界面允许临时改）。
    #[serde(default)]
    pub chara: String,
}

/// 目的：配置界面里的一份权限视图（生效态 + 可选档位 + 收窄提示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PermissionView {
    /// 目的：生效档位 id。
    pub profile: crate::units::pure::permission::api::ProfileId,
    /// 目的：用户标签。
    pub label: String,
    /// 目的：一句话说明。
    pub summary: String,
    /// 目的：生效的共享区归属（读 / 写范围与黑名单）。
    pub shared: PermissionSharedView,
    /// 目的：这条会话是不是子会话收窄层。
    pub narrowed: bool,
    /// 目的：父档位（子会话）。
    pub parent_profile: Option<crate::units::pure::permission::api::ProfileId>,
    /// 目的：可选档位（前端照抄，不自己编）。
    pub profiles: Vec<crate::units::pure::permission::api::ProfileSpec>,
}

/// 目的：权限视图里的共享区归属。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PermissionSharedView {
    /// 目的：读范围。
    pub read: crate::units::workspace::api::ScopeMode,
    /// 目的：写范围（提交的准入门槛）。
    pub write: crate::units::workspace::api::ScopeMode,
    /// 目的：黑名单（全局硬底线 + 会话追加）。
    pub deny: Vec<String>,
}

/// 目的：会话配置视图（配置界面用）：身份与冻结标记 + 可改项 + 运行能力事实。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionConfig {
    pub sid: String,
    /// 目的：会话类型名（声明里的，如「单机」「会议」）。
    pub kind: String,
    /// 目的：会话已经开过（流水里有内容）= agent 名单与形态冻结。
    pub started: bool,
    pub agents: Vec<ConfigAgent>,
    pub tier: String,
    /// 目的：档位是否随主会话（子会话为 true）：界面不渲染档位选择，后端拒绝子会话改档位。
    pub tier_locked: bool,
    /// 目的：这条会话选的虚拟根名（本机档为空）。
    pub guest: Option<String>,
    /// 目的：本机有哪些虚拟根可选（含"装没装齐"）——界面据此渲染下拉，不自己编列表。
    pub guests: Vec<crate::units::provision::api::GuestBase>,
    /// 目的：当前生效的网络开关（全局策略，只读展示）。
    pub net: bool,
    /// 目的：当前生效的会话权限（档位 / reach / decision / 共享区范围）。
    pub permission: PermissionView,
    /// 目的：思考强度（工作根会话级）：当前档 + 可选档与不可用原因。
    pub thinking: ThinkingView,
    /// 目的：过期内容替换开关（工作根会话级）：配置界面的「上下文」区据此画勾。
    pub context_freshness: bool,
    /// 目的：过期内容保底百分比（0 = 不保底；只写工作根）。
    pub freshness_keep_percent: u8,
    pub pins: BTreeMap<String, String>,
    /// 目的：本机能不能承载**当前档位**（虚拟机档的前置条件）：界面据此决定虚拟机档能不能选。
    pub tier_ready: bool,
    /// 目的：承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// 目的：**虚拟机档**能不能选（与当前档位无关）：为假时界面禁用虚拟机档，编辑与「开始」也会拒绝。
    pub vm_available: bool,
    /// 目的：虚拟机档为什么不能选（`vm_available` 为真时为空）。
    pub vm_unavailable_reason: String,
    /// 目的：虚拟机档的**逐项前置**（缺哪几项、每项怎么补）：界面照抄，不自己编话。
    pub vm_requirements: Vec<crate::units::vm::api::VmRequirement>,
    /// 目的：运行能力报告（模块声明 / 包库可用 / 缺包 / 虚拟机档诊断 / 拒收原因）。
    pub runtime: RuntimeReport,
    /// 目的：依赖文件夹（把运行包放进这里；真实路径，给用户看）。
    pub runtimes_dir: String,
}

/// 目的：编辑提交（配置界面用）：改模块、模型、档位、定版与网络。
///   名字与形态不在这里——会话一旦开过（流水有内容）它们就冻结了。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEdit {
    pub agents: Vec<ConfigAgent>,
    pub tier: String,
    #[serde(default)]
    pub guest: Option<String>,
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    /// 目的：会话级网络档位层（根 ceiling / 子 narrowing）。`None` = 保留现值。
    #[serde(default)]
    pub permission: Option<crate::units::pure::permission::api::PermissionLayer>,
    /// 目的：会话级共享区归属（根覆盖 / 子收窄）。`None` = 保留现值。
    #[serde(default)]
    pub shared: Option<crate::units::workspace::api::SharedEdit>,
}

/// 目的：上下文「继续」只在后端判断确实有断点时出现（单 agent 上一轮被打断 / 协作泵没走完）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContinueView {
    /// 目的：断点种类：`turn`（上一轮被打断，继续重开一轮）/ `collab`（恢复协作）。
    pub kind: String,
    /// 目的：按钮文字（后端给，前端不按 kind 自己编）。
    pub label: String,
    /// 目的：为什么出现（按钮 title 与续跑记录用）。
    pub reason: String,
}

/// 目的：会话列表视图。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionView {
    pub sid: String,
    /// 目的：会话类型名（落盘声明里的那个标签）。
    pub kind: String,
    /// 目的：这条会话有没有流程（有流程就不接受自由发言，由机制推进）。
    pub flows: bool,
    /// 目的：决定权是不是整块交给核心（没有名单、没有需求：界面叫代理式）。
    pub auto: bool,
    pub done: bool,
    /// 目的：这条会话记的执行档位（`host` / `vm`）：打开时据此提示"环境已变"。
    pub tier: String,
    /// 目的：记的档位**现在还能不能承载**（本机档恒真）。为假时前端打开前给提示，但不拦打开。
    pub tier_ready: bool,
    /// 目的：承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// 目的：生效权限档位 id（状态栏显示；子会话可能是收窄后的档）。
    pub permission_profile: String,
    /// 目的：生效权限档位标签。
    pub permission_label: String,
    /// 目的：思考强度（工作根会话级）：状态栏 chip 的文字与菜单都用它。
    pub thinking: ThinkingView,
    /// 目的：过期内容替换开关（工作根会话级）：状态栏「工具」菜单据此画勾。
    /// 约束：总是出现（false = 关）；子会话沿工作根共享同一个值。
    pub context_freshness: bool,
    /// 目的：过期内容保底百分比（0 = 不保底）：与开关一样是工作根会话级。
    pub freshness_keep_percent: u8,
    /// 目的：最近一次发送视图的替换统计（判了几条、换了几条、省多少 token）。
    /// 约束：只活在活会话里——重建后为 None，面板就不显示这一行。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_stats: Option<crate::units::conversation::session::api::FreshnessStats>,
    /// 目的：这条会话是不是带收窄层（子会话）。
    pub permission_narrowed: bool,
    /// 目的：子会话的父档位（根为空）；权限快速切换据此禁用更高档。
    pub permission_parent: Option<String>,
    /// 目的：**这条会话此刻在跑吗**（生成中）：界面据此把「发送/继续」换成「停止」并显示占位动画。
    ///   它是**对账副本**：运行态的唯一真相是推的 `SessionEvent::Working`（短暂、不落盘）；
    ///   这份快照供界面在"本页对这条会话还没有实时知识"时对齐——刚刷新页面、事件台裁掉一段后
    ///   重连、别人建的会话（见 docs/session/session-model.md 二之二）。
    pub running: bool,
    /// 目的：**这条工作有「本次需求」吗**：前端据此决定要不要渲染「改需求」按钮——
    ///   没有就**根本不渲染**（不是灰着）。这是领域事实（有没有需求行），不是"模式"。
    pub can_update_task: bool,
    /// 目的：运行态（`active` / `stopped` / `archived`）：**持久事实**，与短暂的 `running` 分开。
    ///   界面据此标出"已暂停 / 已关闭"（这类会话不会再被派发或唤醒）。
    pub run: String,
    /// 目的：当前等用户裁决的**队首那张**（None = 没有）：**快照形态**，与推的 `decision_card` 同源。
    ///   刷新页面时界面照样画得出那张卡；推的那条只是增量（队列里后面的几张在 waiting 里）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<serde_json::Value>,
    /// 目的：上下文「继续」按钮（None = 不显示）；前端只按它渲染，不自己判断末条/协作泵。
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_action: Option<ContinueView>,
    /// 目的：最近一次模型调用的上下文用量（只对 `Session::Single` 有：single / proxy / agent 子会话）。
    /// 约束：协作主会话没有一条持续发给模型的对话，因此没有这个字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::units::transcript::api::ContextUsage>,
    /// 目的：这条会话**累计**的模型花费（跨回合、跨压缩；回档不清零）：面板显示累计输入 / 输出与缓存命中。
    /// 约束：总是出现（全零 = 还没调用过模型）；只累计供应商精确报回的调用，估算不进。
    pub usage_totals: crate::units::conversation::session::api::UsageTotals,
    /// 目的：最近一次 shell 的运行状态（状态栏卡片）；没跑过 shell 就是 null。
    /// 约束：总是出现（null = 没有）——前端据此清掉回档已经截掉的旧快照。
    pub shell: Option<crate::units::conversation::session::api::ShellLive>,
    /// 目的：外部工作区根；None = 产品内布局。界面据此显示“删除只删记录”、工作区标记。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// 目的：外部工作区根不可达（界面禁止打开 / 不渲染启动按钮）。
    #[serde(default)]
    pub workspace_missing: bool,
}

/// 目的：思考强度视图：当前选择 + 共同默认档 + 可选档（状态栏与配置页共用一份事实）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThinkingView {
    /// 目的：会话自己的选择（None = 没选过，跟随模型默认）。
    pub level: Option<String>,
    /// 目的：这组模型共同的默认档（None = 没有统一默认：各模型各自默认或都不指定）。
    pub default_level: Option<String>,
    /// 目的：给人看的当前档（会话选择，否则共同默认；没有可显示的档时是「各模型默认 / 未设置」）。
    pub label: String,
    /// 目的：可选档（只列这组模型都登记了的档；按全局目录顺序）。
    pub options: Vec<OptionView>,
    /// 目的：现在这组模型/目录有什么要注意的（空 = 正常）。
    pub note: String,
}

/// 目的：会话文件清单视图（前端 @ 菜单与「长路径缩写」用）：相对清单 + 真实根。
///   根一律是 slash() 书写形式（/ 分隔、无扩展长度前缀）；拿不到根就给空串（前端原样显示，不猜）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilesView {
    pub work: Vec<String>,
    pub agents: Vec<FilesAgentView>,
    pub roots: FilesRootsView,
    /// 目的：工作区用量（文件数与总字节）：删除前如实交代"还有多少东西会被一起删"。
    pub usage: crate::units::workspace::api::WorkUsage,
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

/// 目的：一次 guest 启动的全部参数（核心准备好，调用方线程执行）。
/// 约束：字段全部是拥有所有权的数据或 `Arc`——它要跨线程，不能借用核心状态。
pub struct GuestBoot {
    pub(crate) vm: Arc<dyn Vm + Send + Sync>,
    pub(crate) root: PathBuf,
    pub(crate) probe: Arc<dyn HostProbe + Send + Sync>,
    pub(crate) log: Arc<dyn Log + Send + Sync>,
    /// 目的：同时台上限（0 = 不限制）；到上限在 run 里如实拒绝。
    pub(crate) max_guests: u32,
    /// 目的：用户授权的主机只读根（进围栏规格，与工具执行那一份同源）。
    pub(crate) host_read_roots: Vec<PathBuf>,
    pub(crate) engine_path: Option<String>,
    pub(crate) share_path: Option<String>,
    pub(crate) shares: Vec<&'static str>,
    pub(crate) lease: String,
    /// 目的：这条会话要拉起的 guest 规格里与会话有关的那几项。
    pub(crate) net: bool,
    pub(crate) plan: ExecPlan,
    pub(crate) sandboxes: Sandboxes,
    /// 目的：拉起之前先停掉同一租约已有的 guest（改配要重建）。
    pub(crate) restart: bool,
}

/// 目的：改配时要等 guest 的那一段——切回本机档只停；切入或留在虚拟机档要起（可能先停旧的）。
pub enum EditGuest {
    /// 起或重建 guest（`restart` = 先停同一租约的旧 guest）。
    Boot(Box<GuestBoot>),
    /// 切回本机档：把旧的 guest 销毁。
    Stop(GuestStop),
}

/// 目的：一次 guest **只读诊断**（慢：要在 guest 里采两轮 /proc 快照）——同样在调用方线程上跑。
/// 约束：只读——不写 guest、不改会话状态；采集与解析的细节在 vm 能力里。
pub struct GuestDiagnose {
    pub(crate) vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：这条会话要诊断的那台 guest 的租约。
    pub(crate) lease: String,
}

/// 目的：一次"启用会话"的两段——核心只判断与准备（可选的一段 guest 启动），慢的那一段在调用方线程上跑。
/// 约束：两个变体都装箱——SessionView / GuestBoot 都比指针大得多，装箱让两个分支体量一致。
pub(crate) enum PreparedActivation {
    /// 会话正在生成：附着即可，不重建、不重启 guest。
    Running(Box<SessionView>),
    /// 可以启用：可选 guest 启动；提交时重建会话并解冻整棵子树。
    Ready(Box<ReadyActivation>),
}

/// 目的：可启用会话的准备结果（装箱后仍要能逐个取字段）。
pub(crate) struct ReadyActivation {
    /// 目的：要启用的会话 id。
    pub(crate) sid: String,
    /// 目的：这一趟要跑的 guest 启动（没有 = 本机档）。
    pub(crate) boot: Option<GuestBoot>,
    /// 目的：本次启动要占的租约（提交失败时据此收掉刚拉起的 guest）。
    pub(crate) lease: Option<String>,
}

/// 目的：工作方法族与两段结果（实现在 `service`，入站面只经这里取）。
pub(crate) use crate::units::work::service::{PreparedEdit, PreparedWork, Work};

/// 目的：建代理会话要落进 `meta` 的那几项——工作名、授予时刻、档位、guest 规格与外部工作区。
pub(crate) struct ProxySeed<'a> {
    /// 目的：新会话的工作名，也是它在会话目录里的落盘名。
    pub(crate) name: &'a str,
    /// 目的：会话类型名（要在类型表里；解析成落盘声明）。
    pub(crate) kind: &'a str,
    /// 目的：这条会话的静止席位 id（代理式会话坐核心代理席，由类型声明解析出来）。
    pub(crate) seat: &'a str,
    /// 目的：授予的秒级时间戳，写进 `meta.delegation`。
    pub(crate) granted_at: i64,
    /// 目的：用户选的档位；也是这条会话建子工作时的默认档位。
    pub(crate) tier: Tier,
    /// 目的：虚拟机档要用的 guest 规格（空串按没给）。
    pub(crate) guest: Option<&'a str>,
    /// 目的：权威档的权限档位。
    pub(crate) profile: crate::units::pure::permission::api::ProfileId,
    /// 目的：外部工作区绑定（已解析，不是原始路径）。
    pub(crate) workspace: Option<crate::units::conversation::session::api::WorkspaceBinding>,
    /// 目的：创建时定的过期内容替换（None = 关）：`(开, 保底百分比)`，写进新会话的工作根 meta。
    pub(crate) freshness: Option<(bool, u8)>,
}

/// 目的：工作的入站契约：建工作、会话配置与编辑、文件与提交历史，以及运行报告与核心推荐。
///   呈现层按屏幕只取这一面；启用 / 改需求 / 回档 / 停止在 `runtime::api::RuntimeOps`。
pub trait WorkOps: Send + Sync {
    /// 目的：建工作——回包是（会话与名单）+ **事件台头部**，开场事实只进事件台（命令不携带事实）。
    /// 约束：呈现侧建会话统一走动作表（`create_session`）；这一格保留给契约测试与其它调用方，
    /// 二进制 crate 里没有调用点会被 dead_code 误报（见 docs/testing/quality-isolation.md 的 allow 清单）。
    #[allow(dead_code)]
    fn create_work(&self, spec: WorkSpec) -> Result<(WorkOpened, u64), String>;
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
    ) -> Result<crate::units::workspace::api::WorkLog, String>;
    /// 目的：一条提交的详情（只读）。
    fn work_commit_view(
        &self,
        sid: &str,
        commit: u64,
    ) -> Result<crate::units::workspace::api::CommitDetail, String>;
    /// 目的：一条提交里某个文件的内容预览（只读；路径必须在这条提交的树里）。
    fn work_commit_file(
        &self,
        sid: &str,
        commit: u64,
        path: &str,
    ) -> Result<crate::units::workspace::api::CommitFileView, String>;
    #[allow(dead_code)]
    fn exists(&self, sid: &str) -> Result<bool, String>;
    /// 目的：**在世会话 × 历史的并集**（界面上的会话列表）：只有会话中心同时知道两边，所以归这里。
    /// 落盘历史的列表 / 打开 / 删除归 `session::api::HistoryOps`。
    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String>;
    /// 目的：运行包与档位的运行报告（只报事实）。
    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String>;
    /// 目的：新建工作时的**档位选择**（默认档 + 虚拟机档可用性与逐项前置）。
    /// 与「开始」的校验同源（同一把 `tier_readiness` 尺子），界面照抄，不自己编话。
    fn tier_choices(&self) -> Result<TierChoices, String>;
    /// 目的：核心按任务推荐的 agent 草案（带理由；用户可改）。
    fn suggest_models(&self, task: &str, kind: &str) -> Result<Vec<AgentSuggestion>, String>;
    /// 目的：会话类型表给界面看的那份（选类型时的名字与说明、要不要名单与需求）。
    /// 约束：按 systools/profiles.yaml 的声明顺序；界面渲染它，不自己复制类型知识。
    fn session_types(&self) -> Result<Vec<SessionTypeView>, String>;
    /// 目的：临时拼装的白名单：只认已注册的流程 id，重复给同一个 id 如实拒绝。
    fn flow_ids(&self, ids: &[String]) -> Result<Vec<String>, String>;
    /// 目的：把启动目录收成外部工作区根（"在哪个目录启动就用哪个目录"，建会话前判一次）。
    /// 约束：与建会话时的 `resolve` 同一条阶梯但**不探可写**；不建目录、不落盘。
    ///   Err 是「为什么不能当工作区」的整句文案，调用方据此回退到产品内 work/ 并说明。
    fn ambient_workspace(&self, raw: &std::path::Path, tier: Tier) -> Result<PathBuf, String>;
}

/// 目的：一个会话类型在界面上的样子（新建向导的逐个选项）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionTypeView {
    /// 目的：类型 id（提交与命令用它）。
    pub kind: String,
    /// 目的：显示名（界面按它渲染）。
    pub name: String,
    /// 目的：一句说明（这个类型与别的有什么区别）。
    pub hint: String,
    /// 目的：有没有流程（有 = 由流程推进，名单可以先空着让核心拟）。
    pub flows: bool,
    /// 目的：要不要选 agent 名单（代理式不要：决定权整块交给核心）。
    pub roster: bool,
    /// 目的：要不要写本次需求（代理式不要：目标由核心自己拆）。
    pub needs_task: bool,
    /// 目的：这个类型的流程至少要几个 agent（选名单的地方按它凑人；没有流程就是 0）。
    pub min_agents: usize,
}
