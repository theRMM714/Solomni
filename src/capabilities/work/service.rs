//! 目的：work 的核心层——`Work` 门面、方法族分区与跨分区共用的判据。
//! 管：注入的端口与处理器钩子、建造入口 `new`、只读访问器（清单扫描 / 保留名 / 虚拟根 / 落盘），
//!   建工作与改配的中间形状（`PreparedWork` / `PendingWork` / `PreparedEdit`）与运行包重建判据。
//! 不管：端口怎么实现（各能力 detail）、事件台与命令队列（`conductor`）、会话本体（`turn`）。
//! 联动：`api` 是唯一出口；`conductor` 装配时把核心自有工具的处理器作为钩子注入，落盘策略走 `session`。

mod action;
mod activation;
mod boot;
mod config;
mod create;
pub(crate) mod guest_job;
mod host;
mod model;
mod report;
pub(crate) mod suggest;
mod views;

use crate::capabilities::collab::api::CollabSession;
use crate::capabilities::llm::api::{Channel, Llm};
use crate::capabilities::modules::api::{Module, Modules, Roster};
use crate::capabilities::packages::api as pkgs;
use crate::capabilities::packages::api::Packages;
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::provision::api::{GuestBase, Provision};
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{
    AgentMeta, History, HistoryView, RunState, SessionEvent, SessionMeta,
};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::turn::api::{Session, ToolHandlers, Turn};
use crate::capabilities::vm::api::Vm;
use crate::capabilities::work::api::{
    AgentInstance, AgentSuggestion, ConfigAgent, ContinueView, FilesAgentRootView, FilesAgentView,
    FilesRootsView, FilesView, RuntimeReport, SessionConfig, SessionEdit, SessionView,
    ThinkingOptionView, ThinkingView, TierChoices, WorkMode, WorkOpened, WorkSpec,
};
use crate::capabilities::work::domain::{
    dedup_modules, merge_into_one, mode_str, validate_work_name,
};
use crate::capabilities::workspace::api::{Sandbox, Sandboxes, Workspace};
use crate::kernel::ports::{HostProbe, Log};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::work::service::guest_job::{EditGuest, GuestBoot, GuestStop};

/// 目的：核心自有工具的处理器注入点——实现留在 `conductor`，本能力只按放行的工具 id 问"谁认领"。
type Handlers = Box<
    dyn Fn(&Arc<dyn Workspace + Send + Sync>, &Sandbox, &[String]) -> ToolHandlers + Send + Sync,
>;
/// 目的：一次创建的准备结果——"已经建完"或"待提交"（后者等调用方线程把 guest 起起来）。
pub(crate) enum PreparedWork {
    /// 已经建完（代理形态：没有 guest 可等）。
    Ready(WorkOpened),
    /// 目的：待提交——核心已占名并备好工作区与计划，慢的那一段由调用方跑；装箱避免枚举被最大变体撑大。
    Pending(Box<PendingWork>),
}

/// 目的：一次创建的"待提交"准备结果。
pub(crate) struct PendingWork {
    meta: Box<SessionMeta>,
    mode: WorkMode,
    metas: Vec<AgentMeta>,
    task: Option<String>,
    delegate: bool,
    name: String,
    agent_names: Vec<String>,
    sandboxes: Sandboxes,
    /// 每个 agent 已解析好的通道（提交段直接装配，不再可能因"没模型"失败）。
    channels: BTreeMap<String, Channel>,
    /// 协作会话核心自己的通道（协作必须有；单 agent 未用核心默认时为空）。
    core_channel: Option<Channel>,
    /// 失败时要撤掉的工作区空目录（只对顶层会话给：子会话与父共用目录）。
    discard: Option<(String, Vec<String>)>,
}

/// 目的：一次启用的准备结果（两段之间的交接物）：附着到正在生成的会话，或带一段待跑的 guest 启动。
pub enum PreparedActivation {
    /// 会话正在生成：直接附着，返回它此刻的视图。
    Running(Box<SessionView>),
    /// 要提交：guest 启动盘（可选）与租约，等调用方线程把 guest 起起来。
    Ready(Box<ReadyActivation>),
}

/// 目的：`PreparedActivation::Ready` 的载荷。
pub struct ReadyActivation {
    /// 目的：会话 id。
    pub(crate) sid: String,
    /// 目的：待跑的 guest 启动盘（没有 guest 时为 `None`）。
    pub(crate) boot: Option<GuestBoot>,
    /// 目的：共享 guest 租约（没有 guest 时为 `None`）。
    pub(crate) lease: Option<String>,
}

/// 目的：一次改配的准备结果——新的 meta 与要同步的子树（落盘用）。
/// 约束：围栏授权是按次的（调用结束即撤），改配不再按旧配置撤权，所以这里不留旧值。
pub(crate) struct PreparedEdit {
    pub(crate) sid: String,
    pub(crate) new_meta: SessionMeta,
    pub(crate) new_children: Vec<SessionMeta>,
}

/// 目的：工作用例的入口：持有注入的端口与核心自有工具的处理器钩子，方法族按用例分区。
///  只持"真用到"的依赖，不持业务状态：会话本体经方法参数 `turn` 传入，登记处经 `registry` 传入。
pub struct Work {
    /// 产品根：虚拟根这类"装在项目内"的东西按它解析成绝对路径（`vm::resolve_guest`）。
    root: PathBuf,
    /// 会话历史直连面（不持它的端口，R12）：读元信息、读事件流。
    history: Arc<dyn History + Send + Sync>,
    /// 工作区用例面（不持它的端口，R12）：工作区目录、文件、提交与恢复。
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 虚拟机档用例面（不持它的端口，R12）：guest 生命周期与档位就绪检测。
    vm: Arc<dyn Vm + Send + Sync>,
    /// 运行包能力（不持它的端口，R12）：包库、选型诊断与计划。
    packages: Arc<dyn Packages + Send + Sync>,
    /// 依赖补齐面（不持它的端口，R12）：虚拟根清单与装没装齐从它读。
    provision: Arc<dyn Provision + Send + Sync>,
    /// 模块能力（不持它的端口，R12）——模块清单事实的唯一上游。
    modules: Arc<dyn Modules + Send + Sync>,
    /// llm 用例面：按解析出来的通道造收发句柄（通道的解析在登记处能力）。
    llm: Arc<dyn Llm + Send + Sync>,
    /// 提示词册能力：按名字取段（册子本体只有一处）。
    prompt: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面：授权两张表。
    systools: Arc<dyn Tools>,
    /// 工具执行面（不持它的端口，R12）：释放围栏授权走它。
    tools: Arc<dyn ToolExec + Send + Sync>,
    /// 日志端口：失败如实记。
    log: Arc<dyn Log + Send + Sync>,
    /// 宿主能力探测（读环境、查路径存在性都在它后面；本能力因此不碰 std::env 与文件系统）。
    probe: Arc<dyn HostProbe + Send + Sync>,
    /// 项目内基础工具链的只读安装根（python / node / cc）：会话装配时按它放行。
    base_roots: Vec<PathBuf>,
    /// 核心自有工具的处理器（`conductor` 注入）：装配工具面时按放行的 id 现算。
    handlers: Handlers,
}

impl Work {
    /// 目的：装配入口：只收端口与处理器钩子；工作形态的业务状态一个都不留。
    /// 参数：`root` = 产品根；其余为各能力的入站面；`handlers` 为实现留在 `conductor` 的钩子。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        packages: Arc<dyn Packages + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        provision: Arc<dyn Provision + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        prompt: Arc<dyn Prompt>,
        systools: Arc<dyn Tools>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        probe: Arc<dyn HostProbe + Send + Sync>,
        handlers: Handlers,
    ) -> Work {
        Work {
            root,
            history,
            workspace,
            vm,
            packages,
            modules,
            provision,
            llm,
            prompt,
            systools,
            tools,
            log,
            probe,
            base_roots: Vec::new(),
            handlers,
        }
    }

    /// 目的：注入项目内基础工具链的只读安装根；组合根读到 active.json 后调用。
    pub(crate) fn set_base_roots(&mut self, roots: Vec<PathBuf>) {
        self.base_roots = roots;
    }

    /// 目的：宿主能力探测面（会话重建要它认虚拟根与可执行文件）。
    pub(crate) fn probe(&self) -> &dyn HostProbe {
        self.probe.as_ref()
    }

    /// 目的：清单即事实：每次调用重扫（策略在本单元，机制在 ModuleSource）。
    pub(crate) fn scan(&self) -> Roster {
        self.modules.roster()
    }

    /// 目的：本机可选的虚拟根清单（会话配置与创建向导用）。
    /// 约束：清单事实来自依赖补齐能力；补齐面没接入时如实返回空表（界面据此说"没有可选虚拟根"）。
    pub(crate) fn guest_bases(&self) -> Result<Vec<GuestBase>, String> {
        self.provision.guest_bases()
    }

    /// 目的：目录保留名（`systools/names.yaml`）：agent 实例名不得占用（与工作区布局同源）。
    pub(crate) fn reserved_names(&self) -> Vec<String> {
        self.systools.reserved_names()
    }

    /// 目的：事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    pub(crate) fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        crate::capabilities::session::api::record_events(
            self.history.as_ref(),
            self.log.as_ref(),
            sid,
            events,
        );
    }
}

/// 目的：模块声明的能力（模块 id → 能力名，升序；去重）——报告 metadata。
fn declared(modules: &[Module]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in modules {
        if m.manifest.runtimes.is_empty() {
            continue;
        }
        let mut caps = m.manifest.runtimes.clone();
        caps.sort();
        caps.dedup();
        out.insert(m.manifest.id.clone(), caps);
    }
    out
}

/// 目的：新子会话要不要为运行包重建共享 guest——只有活 guest 且它缺其中某个包时才要。
pub(crate) fn needs_guest_packages_rebuild(
    active: bool,
    want: &[String],
    have: Option<&[String]>,
) -> bool {
    if !active {
        return false;
    }
    match have {
        Some(have) => want.iter().any(|p| !have.contains(p)),
        None => !want.is_empty(),
    }
}

/// 目的：guest 装配只认"agent → 模块集合"与环境；模型 / 权限变化不该触发重建。
fn agent_modules(meta: &SessionMeta) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for a in &meta.agents {
        let mut modules = a.modules.clone();
        modules.sort();
        modules.dedup();
        out.insert(a.name.clone(), modules);
    }
    out
}

/// 目的：这次改配要不要重建共享 guest——模块集合或 ExecSpec（档位 / 虚拟根 / 定版）变了才要。
/// 网络是全局策略；全局变更走整体重启，不在这里触发按会话重建。
fn guest_assembly_changed(old: &SessionMeta, new: &SessionMeta) -> bool {
    old.exec.tier != new.exec.tier
        || old.exec.guest != new.exec.guest
        || old.exec.pins != new.exec.pins
        || agent_modules(old) != agent_modules(new)
}
