//! 目的：work 能力的方法族入口——`Work` 的门面（字段、构造）与跨子模块共用的形状与判据。
//! 管：`Work` 的字段与构造、建工作的两段结果、改配的准备结果、模块声明的能力表、运行包重建判据。
//! 不管：报告与档位（`report.rs`）、配置读写（`config.rs`）、调用参数（`model.rs`）、宿主工具路径（`host.rs`）、视图（`views.rs`）、建工作与 guest 启动（`create.rs`）、启用与拟名单（`activation.rs` / `suggest.rs`）、guest 任务本身（`guest_job.rs`）。
//! 联动：runtime 与呈现层经这里的方法名调用；各子模块 impl 同一个 `Work`。
//!   `Turn` / `Registry` / `HostProbe` 都是方法形参、不是字段：本单元不持第二份 `Turn`。

mod activation;
mod config;
mod create;
pub(crate) mod guest_job;
mod host;
mod model;
mod report;
mod suggest;
mod views;

use crate::kernel::ports::{HostProbe, Log};
use crate::units::business::catalog::modules::api::{Module, Modules};
use crate::units::business::catalog::packages::api as pkgs;
use crate::units::business::catalog::packages::api::Packages;
use crate::units::business::catalog::registry::api::Registry;

use crate::units::business::llm::api::{Channel, Llm};
use crate::units::business::prompt::api::Prompt;
use crate::units::business::provision::api::Provision;
use crate::units::business::session_model::api::{Delegation, HistoryView, RunState, SessionMeta};
use crate::units::business::tools::api::{ToolExec, Tools};
use crate::units::business::transcript::api::{OptionView, SessionEvent};
use crate::units::business::turn::turn::api::PROXY_PRIVATE_DIR;
use crate::units::business::turn_state::api::{Gates, History};
use crate::units::business::turn_state::api::{Session, Turn};
use crate::units::business::vm::api::Vm;
use crate::units::business::work::api::{
    AgentSuggestion, ConfigAgent, ContinueView, EditGuest, FilesAgentRootView, FilesAgentView,
    FilesRootsView, FilesView, GuestBoot, PermissionSharedView, PermissionView, PreparedActivation,
    ProxySeed, ReadyActivation, RuntimeDiagnosis, RuntimeReport, SandboxChoices, SessionConfig,
    SessionEdit, SessionTypeView, SessionView, ThinkingView, WorkOpened, WorkSpec,
};
use crate::units::business::work::domain::{merge_into_one, session_started, validate_work_name};
use crate::units::business::workspace::api::Workspace;
use crate::units::pure::registry_model::api::AgentMeta;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：**工作与会话配置的用例面**：建工作 / 改配 / 视图 / 报告 / guest 起停都经它。
///  与 `Core` 分开：这里只管"一场工作怎么被建立、改配、看、报"，
///  不看命令队列、不看协作链推进、不看回档——那些留在 runtime（环 3）。
///  组合根在 `Core::new` 里造一份，与 runtime 共享同一批入站面。
pub struct Work {
    /// 产品根：虚拟根这类"装在项目内"的东西按它解析成绝对路径（`vmstore::api::resolve_guest`）。
    root: PathBuf,
    /// 会话历史直连面（**不持它的端口**，R12）：造/读/写/删会话都走它。
    history: Arc<dyn History + Send + Sync>,
    /// 工作区用例面（**不持它的端口**，R12）：目录布局、沙箱寻址与文件寻址走它。
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 版本库用例面（**不持它的端口**，R12）：提交、拉取、日志与回档走它。
    versions: Arc<dyn crate::units::business::versions::api::Versions + Send + Sync>,
    /// 虚拟机档用例面（**不持它的端口**，R12）：guest 生命周期与就绪检测走它。
    vm: Arc<dyn Vm + Send + Sync>,
    /// 执行环境策略注册表（**值不是端口**）：声明解析成事实、就绪判定与选型派生都经它。
    policies: crate::units::business::sandbox::api::SandboxPolicies,
    /// 工作台策略表（**值不是端口**）：视图里的能力位按会话声明的策略 id 查它。
    bench_policies: Arc<dyn crate::units::business::workbench::api::BenchPolicies>,
    /// 运行包能力（**不持它的端口**，R12）：包库、选型诊断与计划走它。
    packages: Arc<dyn Packages + Send + Sync>,
    /// 依赖补齐面（**不持它的端口**，R12）：虚拟根清单与装没装齐从它读。
    provision: Arc<dyn Provision + Send + Sync>,
    /// 目的：模块子业务（**不持它的端口**，R12）——模块清单事实的唯一上游。
    modules: Arc<dyn Modules + Send + Sync>,
    /// llm 用例面：按解析出来的通道造收发句柄。
    llm: Arc<dyn Llm + Send + Sync>,
    /// 工具执行面（**不持它的端口**，R12）：跑外部/内置工具、释放围栏授权都走它。
    tools: Arc<dyn ToolExec + Send + Sync>,
    /// 提示词单元：只按名字取段。
    prompt: Arc<dyn Prompt>,
    /// 工具总表与机制席位的入站面（授权两张表）：表本体在工具单元里，与册子互不依赖。
    systools: Arc<dyn Tools>,
    log: Arc<dyn Log + Send + Sync>,
    /// 目的：流程工厂（组合根注入）：建工作按会话类型表里的流程序列逐段造，本层不认识任何具体流程。
    flows: Arc<dyn crate::units::business::turn_state::api::Flows>,
}

impl Work {
    /// 组合根专用：与 `Core` 共享同一批入站面（不新建任何实现）。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        versions: Arc<dyn crate::units::business::versions::api::Versions + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        policies: crate::units::business::sandbox::api::SandboxPolicies,
        bench_policies: Arc<dyn crate::units::business::workbench::api::BenchPolicies>,
        packages: Arc<dyn Packages + Send + Sync>,
        provision: Arc<dyn Provision + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        prompt: Arc<dyn Prompt>,
        systools: Arc<dyn Tools>,
        log: Arc<dyn Log + Send + Sync>,
        flows: Arc<dyn crate::units::business::turn_state::api::Flows>,
    ) -> Work {
        Work {
            root,
            history,
            workspace,
            versions,
            vm,
            policies,
            bench_policies,
            packages,
            provision,
            modules,
            llm,
            tools,
            prompt,
            systools,
            log,
            flows,
        }
    }
}

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
    metas: Vec<AgentMeta>,
    task: Option<String>,
    delegate: bool,
    name: String,
    agent_names: Vec<String>,
    sandboxes: crate::units::business::workbench::api::Benches,
    /// 每个 agent 已解析好的通道（提交段直接装配，不再可能因"没模型"失败）。
    channels: BTreeMap<String, Channel>,
    /// 协作会话核心自己的通道（协作必须有；单 agent 未用核心默认时为空）。
    core_channel: Option<Channel>,
    /// 失败时要撤掉的工作区空目录（只对顶层会话给：子会话与父共用目录）。
    discard: Option<(String, Vec<String>)>,
}

/// 目的：一次改配的准备结果——新的 meta 与要同步的子树（落盘用）。
/// 约束：围栏授权是按次的（调用结束即撤），改配不再按旧配置撤权，所以这里不留旧值。
pub(crate) struct PreparedEdit {
    pub(crate) sid: String,
    pub(crate) new_meta: SessionMeta,
    pub(crate) new_children: Vec<SessionMeta>,
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

/// 目的：这次改配要不要重建共享 guest——模块集合或 TargetSpec（档位 / 虚拟根 / 定版）变了才要。
/// 网络是全局策略；全局变更走整体重启，不在这里触发按会话重建。
fn guest_assembly_changed(old: &SessionMeta, new: &SessionMeta) -> bool {
    old.sandbox.policy != new.sandbox.policy
        || old.facts.guest != new.facts.guest
        || old.sandbox.pins != new.sandbox.pins
        || agent_modules(old) != agent_modules(new)
}
