//! 目的：conductor 的核心层——端口抽象、编排业务（会话 / 引擎 / 会话中心）与队列桥。
//! 管：Conductor 门面与各 service 子模块的编排：会话在世表、命令队列、运行态、跨能力用例。
//! 不管：文件读、网络请求、stdin/stdout（机制在各能力的 detail 与 kernel/detail）；装配（new 适配器）只在 main 组合根。
//! 联动：api 层经 ConductorHandle 把命令与事件交给这里；出站动作经 ports::ProxyHost。

use crate::capabilities::builds::api::Builds;
use crate::capabilities::conductor::api::{
    AgentInstance, AgentSuggestion, ConfigAgent, ContinueView, FilesAgentRootView, FilesAgentView,
    FilesRootsView, FilesView, RewindTarget, RuntimeReport, SessionConfig, SessionEdit,
    SessionView, ThinkingOptionView, ThinkingView, TierChoices, WorkMode, WorkOpened, WorkSpec,
};
use crate::capabilities::llm::api::Llm;
use crate::capabilities::packages::api::Packages;
use crate::capabilities::provision::api::Provision;
use crate::capabilities::session::api::History;
#[cfg(test)]
pub(crate) use crate::capabilities::session::api::Live;
use crate::capabilities::session::api::{Answered, GateTicket, SessionEvent};
use crate::capabilities::vm::api::Vm;
use crate::capabilities::workspace::api::Workspace;

use crate::capabilities::collab::api::AfterTurn;
use crate::capabilities::collab::api::CollabSession;
use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::Msg;
use crate::capabilities::modules::api::{Module, Modules};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{AgentMeta, HistoryView, RunState, SessionMeta};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::kernel::api::SessionId;
use crate::kernel::ports::Log;
use compact::{CompactChoice, CompactRuntime};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

/// 目的：会话实例：单 agent 会话或协作会话（本体自带端口，可跨线程移动）。
///  两变体大小差得远（协作会话带整份讨论状态），装箱只换来一次间接寻址、
///  却把"会话本体可直接移动"这个形状改掉——有意不装箱（见 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::large_enum_variant)]
pub enum Session {
    Single(crate::capabilities::session::api::AgentSession),
    Collab(CollabSession),
}

/// 目的：模块列表保序去重（一个 agent 内重复只留一份；跨 agent 的重叠是合法的共用）。
pub(crate) fn dedup_modules(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for m in list {
        if !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// 目的：整张名单的扁平模块清单（保序去重）：同一模块被多个 agent 共用时只出现一次。
pub(crate) fn flat_modules(agents: &[AgentMeta]) -> Vec<String> {
    let mut out = Vec::new();
    for a in agents {
        for m in &a.modules {
            if !out.contains(m) {
                out.push(m.clone());
            }
        }
    }
    out
}

/// 单模式的组合语义：多个 agent 的模块并成一个**临时组合**实例（去重、保序；模型取核心默认）。
/// 人设取空：多份人设无法诚实合并，由用户在会话配置里补。
fn merge_into_one(picked: &[AgentInstance], name: &str) -> AgentInstance {
    let mut merged: Vec<String> = Vec::new();
    for a in picked {
        for id in &a.modules {
            if !merged.contains(id) {
                merged.push(id.clone());
            }
        }
    }
    AgentInstance {
        name: name.to_string(),
        transient: true,
        modules: merged,
        model: None,
        chara: String::new(),
    }
}

/// 目的：一次单 agent 生成的**准备结果**（核心线程上只算到这里，模型调用在工作线程上）。
pub(crate) enum Prepared {
    /// 可以跑：会话已从核心表取出，由工作线程独占。
    Run {
        /// 装箱：这个变体比其它两个大得多（会话本体），而它本来就是**一次性移交**给工作线程的。
        session: Box<crate::capabilities::session::api::AgentSession>,
        /// 本回合的**身份块**：按当前提示词册与登记处现渲染（不进会话的消息列表）。
        identity: String,
        /// 生成前要先给用户的事件（例如工具形态变更的提示）。
        prefix: Vec<SessionEvent>,
        /// 本回合要注入的**派发任务**（节点执行）：在工作线程上先压缩、再注入，任务不会被压掉。
        task: Option<String>,
        llm: crate::capabilities::llm::api::LlmOpts,
        /// 增量落盘手柄：逐轮外送的同时就落盘（中途刷新页面因此看得到已产生的部分）。
        persister: Persister,
    },
    /// 不用跑模型：直接把这批事件回给调用方（例如"末条是 AI 发言"）。
    Immediate(Vec<SessionEvent>),
    /// 不是单 agent 会话：交回调用方走它自己那条路（协作）。
    NotSingle,
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
    pub(crate) boot: Option<crate::capabilities::conductor::service::guest_job::GuestBoot>,
    /// 目的：本次启动要占的租约（提交失败时据此收掉刚拉起的 guest）。
    pub(crate) lease: Option<String>,
}

/// 目的：**应用服务**：持有注入的端口、能力面与会话中心；前端拿不到它（只拿 `conductor::api::Ops`）。
///  组合根在 `ConductorHandle::spawn` 里把它**移进核心自己的执行线程**——此后状态只被那一个线程碰，
///  所以这里不加任何锁（并发不变式见 ARCHITECTURE.md §一）。
pub struct Conductor {
    /// 产品根：虚拟根这类"装在项目内"的东西按它解析成绝对路径（`vm::resolve_guest`）。
    root: std::path::PathBuf,
    /// 登记处能力：**四份 yaml 的状态在它里面**，conductor 只按 `Registry` 调用（看不见它的字段）。
    registry: Box<dyn Registry>,
    /// 会话历史直连面（**不持它的端口**，R12）：造/读/写/删会话都走它。
    history: Arc<dyn History + Send + Sync>,
    /// 工作区用例面（**不持它的端口**，R12）：工作区目录与版本化走它。
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 虚拟机档用例面（**不持它的端口**，R12）：guest 生命周期与档位就绪检测走它。
    vm: Arc<dyn Vm + Send + Sync>,
    /// 运行包能力（**不持它的端口**，R12）：包库、选型诊断与计划走它。
    packages: Arc<dyn Packages + Send + Sync>,
    /// 依赖补齐面（**不持它的端口**，R12）：虚拟根清单与装没装齐从它读。
    provision: Arc<dyn Provision + Send + Sync>,
    /// 目的：模块能力（**不持它的端口**，R12）——模块清单事实的唯一上游。
    modules: Arc<dyn Modules + Send + Sync>,
    /// 目的：构建能力面（**不持它的端口**，R12）——创建会话前补编声明了 `build:` 的模块。
    builds: Arc<dyn Builds + Send + Sync>,
    /// llm 用例面：按解析出来的通道造收发句柄 + 信封的无歧义修复（通道的**解析**在登记处能力）。
    llm: Arc<dyn Llm + Send + Sync>,
    /// 压缩通道工厂：全局设置只在这里对账；会话在压缩触发时现取（不刷新会话）。
    compact: Arc<CompactRuntime>,
    /// 工具执行面（**不持它的端口**，R12）：跑外部/内置工具、释放围栏授权都走它。
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 宿主能力探测（读环境、查路径存在性都在它后面；conductor 因此不碰 std::env 与文件系统）。
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    /// 常驻服务的统一管理面（**不持它的端口**）：动作面与会话回收经它。
    residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    /// 隐秘字段的统一管理面：工具 / 服务起进程时按它解析注入项并脱敏。
    secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    /// 项目内基础工具链的只读安装根（python / node / cc）：shell 的 sandbox 与 external 都放行它们。
    /// 来源是组合根读到的 active.json；不按 agent 的模块重复注入。
    base_roots: Vec<std::path::PathBuf>,
    /// 目的：会话 → shell 实时状态槽（状态栏卡片用）；工具循环写、SessionView 读。
    shell_slots: std::sync::Mutex<
        std::collections::BTreeMap<String, Arc<crate::capabilities::session::api::ShellSlot>>,
    >,
    /// 提示词册能力：**册子本体在它里面**（只有一处），conductor 只按名字取段。
    /// 用 `Arc`：协作会话要与核心**共享**这一份（逐处克隆整本册子是白费）。
    prompt: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面（授权两张表；名字表由组合根另取）：**表本体在工具能力里**，与册子互不依赖。
    systools: Arc<dyn Tools>,
    sessions: HashMap<SessionId, Session>,
    /// **裁决队**（按会话 id 取）：核心各关卡与工具级确认共用同一条队（见 docs/session/session-model.md）。
    /// 它跨线程：生成线程等在工作线程上，回答与快照在核心线程上——同一份句柄、同一把锁。
    desk: Arc<crate::capabilities::session::api::DecisionDesk>,
    /// 正在生成的会话：对象被工作线程**取走**了，核心表里暂时没有它。
    /// 为什么取出而不是就地生成：生成要跑几十秒到几分钟，占着唯一的命令队列会让
    /// 读接口（历史列表、状态）与其它会话的命令全排在它后面——界面因此"假死"。
    /// 这一态只表示"不在表里是因为在生成"，不是"不存在"。
    running: std::collections::BTreeSet<SessionId>,
    /// 正在准备中的会话名（创建分两段：核心先占名与预备，慢的那一段在调用方线程上跑）。
    /// 有了它，同名并发创建就不会都通过"名字不存在"那道检查而互相覆盖。
    starting: std::collections::BTreeSet<String>,
}

mod action;
mod activation;
pub(crate) mod ask_user;
mod compact;
mod env;
mod flow;
pub(crate) mod guest_job;
mod history;
pub(crate) mod persist;
pub(crate) use persist::{persist_events, Persister, SYSTEM_SID_SUGGEST};
mod suggest;
#[cfg(test)]
pub(crate) use history::SessionDelete;
pub mod proxy;
mod rewind;
#[cfg(test)]
pub(crate) use rewind::replay_dialogue;
mod turn;
mod work;
#[cfg(test)]
pub(crate) use work::needs_guest_packages_rebuild;
pub(crate) mod work_tools;
pub(crate) mod workspace_root;
impl Conductor {
    /// 组合根专用：main 负责创建适配器并注入；conductor 不自建任何具体实现。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: std::path::PathBuf,
        registry: Box<dyn Registry>,
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        packages: Arc<dyn Packages + Send + Sync>,
        provision: Arc<dyn Provision + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        builds: Arc<dyn Builds + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        prompt: Arc<dyn Prompt>,
        systools: Arc<dyn Tools>,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
        residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
        secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    ) -> Conductor {
        let compact = Arc::new(CompactRuntime::new(Arc::clone(&llm)));
        let c = Conductor {
            root,
            registry,
            history,
            workspace,
            vm,
            packages,
            provision,
            modules,
            builds,
            llm,
            compact,
            tools,
            log,
            probe,
            residents,
            secrets,
            base_roots: Vec::new(),
            shell_slots: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            prompt,
            systools,
            sessions: HashMap::new(),
            desk: crate::capabilities::session::api::DecisionDesk::new(),
            running: std::collections::BTreeSet::new(),
            starting: std::collections::BTreeSet::new(),
        };
        c.sync_compact_choice();
        c
    }

    /// 目的：从登记处重算压缩选择（设置 / 模型 / 供应商变更后调用）。
    /// 约束：只读登记处、只换共享选择；不重建任何会话。
    pub(crate) fn sync_compact_choice(&self) {
        let app = self.registry.app();
        let id = app.compact_model.trim();
        let choice = if id.is_empty() {
            CompactChoice::Session
        } else {
            match self.registry.channel(Some(id)) {
                Ok(channel) => CompactChoice::Model {
                    id: id.to_string(),
                    channel,
                    window: self.registry.context_of(Some(id)),
                },
                Err(e) => CompactChoice::Unavailable(format!("压缩模型 {} 不可用：{}", id, e)),
            }
        };
        self.compact.set(choice);
    }

    /// 目的：注入给新会话 / 重建会话的压缩通道工厂。
    pub(crate) fn compact_factory(
        &self,
    ) -> Arc<dyn crate::capabilities::session::api::CompactChatFactory> {
        self.compact.clone()
    }

    /// 目的：注入项目内基础工具链的只读安装根；组合根读到 active.json 后调用。
    pub fn with_base_roots(mut self, roots: Vec<std::path::PathBuf>) -> Conductor {
        self.base_roots = roots;
        self
    }

    /// 目的：取（必要时创建）一个会话的 shell 实时状态槽；成员工具面与状态栏共用同一份。
    pub(crate) fn shell_slot(
        &self,
        sid: &str,
    ) -> Arc<crate::capabilities::session::api::ShellSlot> {
        let mut map = self.shell_slots.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            map.entry(sid.to_string()).or_insert_with(|| {
                Arc::new(crate::capabilities::session::api::ShellSlot::default())
            }),
        )
    }

    /// 目的：读会话最近一次 shell 的快照；没有运行过就是 None（状态栏据此决定要不要出 chip）。
    pub(crate) fn shell_view(
        &self,
        sid: &str,
    ) -> Option<crate::capabilities::session::api::ShellLive> {
        self.shell_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .and_then(|s| s.get())
    }

    /// 目的：本机可选的虚拟根清单（会话配置与创建向导用）。
    /// 约束：清单事实来自依赖补齐能力；补齐面没接入时如实返回空表（界面据此说"没有可选虚拟根"）。
    pub(crate) fn guest_bases(
        &self,
    ) -> Result<Vec<crate::capabilities::provision::api::GuestBase>, String> {
        self.provision.guest_bases()
    }

    /// 目的：模块的**有效**启用（用户覆盖 ?? 作者默认）。
    pub fn module_enabled(&self, id: &str) -> bool {
        self.modules.module_enabled(id)
    }

    /// 目的：服务的**有效**启用（模块停用 ⇒ false）。
    pub fn service_enabled(&self, module: &str, service: &str) -> bool {
        self.modules.service_enabled(module, service)
    }

    /// 目的：模块能力面（手柄与核心共享同一份；启停写面经它）。
    pub(crate) fn modules_handle(&self) -> Arc<dyn Modules + Send + Sync> {
        Arc::clone(&self.modules)
    }

    /// 目的：构建能力面（手柄与核心共享同一份；创建会话前在调用方线程补编，核心线程不陪着等编译）。
    pub(crate) fn builds_handle(&self) -> Arc<dyn Builds + Send + Sync> {
        Arc::clone(&self.builds)
    }

    /// 目的：常驻服务面（手柄与核心共享同一份；动作与会话回收经它）。
    pub(crate) fn residents_handle(
        &self,
    ) -> Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync> {
        Arc::clone(&self.residents)
    }

    /// 目的：隐秘字段面（手柄与核心共享同一份）。
    pub(crate) fn secrets_handle(
        &self,
    ) -> Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync> {
        Arc::clone(&self.secrets)
    }

    /// 目的：日志端口句柄：入站手柄（conductor::api）与组合根共用同一份事实记录。
    pub fn log_handle(&self) -> Arc<dyn crate::kernel::ports::Log + Send + Sync> {
        Arc::clone(&self.log)
    }

    /// 目的：工具执行面句柄——入站手柄跑"人直接用的模块工具"要用同一份执行面（不重装）。
    pub fn tools_handle(&self) -> Arc<dyn ToolExec + Send + Sync> {
        Arc::clone(&self.tools)
    }

    /// 目的：登记处能力面（只读）：组合根与测试读登记处的事实走这里。
    pub fn registry(&self) -> &dyn Registry {
        self.registry.as_ref()
    }

    /// 目的：登记处能力面（可写）：登记处的用例只经它调用。
    pub fn registry_mut(&mut self) -> &mut dyn Registry {
        self.registry.as_mut()
    }

    /// 目的：登记处写操作的统一出口——写完重算压缩选择，避免汇总模型指向旧通道。
    pub(crate) fn registry_write<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        let out = f(self.registry.as_mut())?;
        self.sync_compact_choice();
        Ok(out)
    }

    /// 目的：目录保留名（`systools/names.yaml`）：agent 实例名不得占用（与工作区布局同源）。
    pub(crate) fn reserved_names(&self) -> Vec<String> {
        self.systools.reserved_names()
    }

    /// 目的：工具总表的声明书（代理执行者在队列桥那一侧校验参数/拼回执要用）。
    pub(crate) fn systools_book(&self) -> crate::capabilities::tools::api::ToolBook {
        self.systools.book()
    }

    /// 生成期间"会话不在表里"的三种进入点共用这一句（错误文案要一致，别处不再各写一份）。
    fn running_refusal(sid: &str) -> String {
        format!("会话 {} 正在生成中：先「停止」或等它结束，再做这一步", sid)
    }

    /// 目的：把单 agent 会话**交给工作线程**（核心表里留"生成中"）。
    ///  取出的窗口内，核心队列是空的——读接口与其它会话的命令因此照常。
    pub(crate) fn take_single(
        &mut self,
        sid: &str,
    ) -> Result<crate::capabilities::session::api::AgentSession, String> {
        // 运行态闸门：暂停 / 关闭的会话不启动任何生成（唤醒与派发都从这里过）。
        self.dispatch_gate(sid)?;
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        match self.sessions.remove(sid) {
            Some(Session::Single(s)) => {
                self.running.insert(sid.to_string());
                Ok(s)
            }
            Some(other) => {
                // 不是单 agent：原样放回，交给协作那条路（B-1 只搬单 agent）。
                self.sessions.insert(sid.to_string(), other);
                Err("该会话不是单 agent 模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：把协作会话**交给工作线程**（核心表里留"生成中"）。
    ///  会话还没装进内存时先从落盘重建：协作的"继续"可能先于"打开"到达。
    pub(crate) fn take_collab(&mut self, sid: &str) -> Result<CollabSession, String> {
        // 同上：运行态闸门是第一道，先于"取出来"。
        self.dispatch_gate(sid)?;
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        if !self.sessions.contains_key(sid) {
            self.ensure_session(sid)?;
        }
        match self.sessions.remove(sid) {
            Some(Session::Collab(c)) => {
                self.running.insert(sid.to_string());
                Ok(c)
            }
            Some(other) => {
                self.sessions.insert(sid.to_string(), other);
                Err("该会话不是协作模式".to_string())
            }
            None => Err("无此会话".to_string()),
        }
    }

    /// 目的：这一段工作已被停 / 已关闭时，把它的裁决队**整队作废**（等待方按「停止 = 拒绝」解开）。
    /// 返回：要外送 / 落盘的作废事件（本来就没挂着卡 = 空）。判据是落盘的运行态（meta.run），不是内存猜测。
    pub(crate) fn void_gates_of(&mut self, sid: &str) -> Vec<SessionEvent> {
        let reason = match self.history.meta(sid).map(|m| m.run) {
            Ok(RunState::Stopped) => "用户按了停止：这一队裁决一律作废（停止 = 拒绝）",
            Ok(RunState::Closed) => "会话已关闭：这一队裁决一律作废",
            _ => return Vec::new(),
        };
        // 判据是**队本身**（核心各关卡与工具级确认排在同一条队上）；没有队 = 本来就没挂着卡。
        let Some(door) = self.desk.peek(sid) else {
            return Vec::new();
        };
        let cards = door.void();
        if cards.is_empty() {
            return Vec::new();
        }
        vec![SessionEvent::DecisionVoid {
            cards,
            reason: reason.to_string(),
        }]
    }

    /// 目的：取这个会话的**裁决队**（核心各关卡与工具级确认共用同一条）。
    /// 约束：新队按转录里已发出的卡号续号——卡号会话内唯一、跨重启稳定（已答过的不重问）。
    pub(crate) fn desk_of(
        &self,
        sid: &str,
    ) -> Arc<crate::capabilities::session::api::DecisionDoor> {
        if let Some(d) = self.desk.peek(sid) {
            return d;
        }
        let issued = self
            .history
            .load(sid)
            .map(|(_, ev)| crate::capabilities::session::api::issued_in(&ev))
            .unwrap_or(0);
        self.desk.session(sid, issued)
    }

    /// 目的：会话没了，它的裁决队随会话一起消失（等待方与转录都不复存在）。
    pub(crate) fn forget_desk(&self, sid: &str) {
        self.desk.forget(sid);
    }

    /// 目的：这些会话里是否有任何一个的回合还攥在生成线程手里（核心里"生成中"）。
    pub(crate) fn running_any(&self, ids: &[String]) -> bool {
        ids.iter().any(|s| self.running.contains(s))
    }

    /// 目的：把 root 及其整棵子树里等用户裁决的队**一律作废**（停止 = 拒绝），按会话给出作废事件。
    /// 约束：会话此刻被生成线程拿在手里时它不在表里——那一份由 put_collab 交回时按运行态作废。
    pub(crate) fn void_gates_in_subtree(&mut self, root: &str) -> Vec<(String, Vec<SessionEvent>)> {
        let mut out = Vec::new();
        for sid in self.subtree_of(root) {
            let evs = self.void_gates_of(&sid);
            if !evs.is_empty() {
                out.push((sid, evs));
            }
        }
        out
    }

    /// 目的：协作生成结束**交回**：重新插入 + 解除"生成中"，并**为就绪节点派发子会话**。
    ///  转录已由工作线程按"一次模型调用"的粒度增量落盘（见 `Persister`），这里不重复落。
    ///  返回值：派发产生的事件（调用方负责入台）。
    pub(crate) fn put_collab(
        &mut self,
        sid: &str,
        c: CollabSession,
    ) -> (Vec<SessionEvent>, Vec<(String, String, String)>) {
        self.running.remove(sid);
        self.sessions.insert(sid.to_string(), Session::Collab(c));
        // 交回时这段工作已被停 / 已关闭：队列里没答的卡**一律作废**（生成期间对象在别人手里，只能在这里补）。
        let mut voided = self.void_gates_of(sid);
        let (spawned, todo) = self.spawn_ready_nodes(sid);
        voided.extend(spawned);
        (voided, todo)
    }

    /// 方案过审后：链里**就绪且还没有子会话**的节点各建一个子会话，并如实外送。
    /// 为什么在这里：会话表只有核心能碰（泵不建会话）；派发是核心的职责。
    /// 子会话的产出 = 它**用 submit_report 工具交的回报**（核心操作走工具调用，不认正文里的 JSON）。
    /// 没交回报就退回"最后一条转录行"（如实取到的东西），仍取不到就说"没有产出"。
    fn node_note(&self, child: &str) -> String {
        let evs = match self.history.load(child) {
            Ok((_, evs)) => evs,
            Err(_) => Vec::new(),
        };
        let mut lines: Vec<String> = Vec::new();
        let mut reported: Option<String> = None;
        for ev in &evs {
            let Some(ls) = ev.get("lines").and_then(|l| l.as_array()) else {
                continue;
            };
            for l in ls {
                if let Some(t) = l.get("line").and_then(|x| x.as_str()) {
                    lines.push(t.to_string());
                }
                let Some(tool) = l.get("tool") else { continue };
                let name = tool.get("name").and_then(|x| x.as_str()).unwrap_or("");
                if name != crate::capabilities::tools::api::REPORT {
                    continue;
                }
                let out = tool
                    .get("output")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !out.trim().is_empty() {
                    reported = Some(out);
                }
            }
        }
        reported
            .or_else(|| lines.last().cloned())
            .unwrap_or_else(|| "（该节点没有产出）".to_string())
    }

    /// 标记节点完成（记下它的产出），供父会话做总验收。
    fn mark_node_done(&mut self, sid: &str, node: &str, note: &str) {
        if let Some(Session::Collab(c)) = self.sessions.get_mut(sid) {
            c.mark_node_done(node, note);
        }
    }

    /// 目的：给压缩回合准备身份块；切分与模型调用都在工作线程上（界面不被阻塞）。
    /// 返回：本回合的身份块。
    /// 错误：会话不在世或不是单 agent 时如实报错。
    pub fn compact_plan(&mut self, sid: &str) -> Result<String, String> {
        // 会话不在世或不是单 agent：**报错**，不给出"空身份块"这种假计划
        // （压缩只对单 agent 会话成立；协作会话由各成员会话按阈值自己压）。
        let model = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.model_id().map(str::to_string),
            _ => {
                return Err(format!(
                    "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                    sid
                ))
            }
        };
        // 手动压缩也要读**当前**设置：用户刚把保留比例调低，不该等会话重建才生效。
        let (window, percent, keep) = self.context_budget(model.as_deref());
        let Some(Session::Single(s)) = self.sessions.get_mut(sid) else {
            return Err(format!(
                "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                sid
            ));
        };
        s.set_context_budget(window, percent, keep);
        Ok(s.params().identity(&*self.prompt, s.tool_mode()))
    }

    /// 目的：往某个会话注入一条**系统消息**（讨论的提醒走这条；见 session-model.md 二"系统消息"）。
    ///  返回要外送/落盘的事件。
    pub fn note_system(&mut self, sid: &str, text: &str) -> Result<Vec<SessionEvent>, String> {
        let mut s = self.take_single(sid)?;
        let events = s.note_system(text);
        self.put_single(sid, s);
        self.persister(sid).persist(&events);
        Ok(events)
    }

    /// 目的：给某个会话接下来的行打上**整场工作的下一个回合 id**（节点执行也用同一套编号）。
    ///  一个 agent 一个会话：它的回合计数来自父会话——回档同步靠两边同一套编号。
    pub(crate) fn bump_turn_of_child(&mut self, child: &str) -> u64 {
        let parent = self
            .history
            .load(child)
            .ok()
            .and_then(|(m, _)| m.parent.clone());
        let Some(parent) = parent else {
            return 0;
        };
        let tid = match self.sessions.get_mut(&parent) {
            Some(Session::Collab(c)) => c.next_turn_id(),
            _ => 0,
        };
        if let Some(Session::Single(s)) = self.sessions.get_mut(child) {
            s.set_turn(tid);
        }
        tid
    }

    // ---- 会话历史 ----
}

// 测试访问器：验证职责提示词已种入历史首条（回归：会话曾丢失 system 提示词）。
#[cfg(test)]
impl Conductor {
    pub fn single_history(&self, sid: &str) -> Option<Vec<Msg>> {
        match self.sessions.get(sid) {
            Some(Session::Single(s)) => Some(s.dialogue().to_vec()),
            _ => None,
        }
    }

    // 测试访问器：读台账（验证"回放 = 实时"：重建后的事实与正文必须逐条一致）。
    pub fn single_reads(
        &self,
        sid: &str,
    ) -> Option<Vec<crate::capabilities::session::api::ReadAt>> {
        match self.sessions.get(sid) {
            Some(Session::Single(s)) => Some(s.reads().to_vec()),
            _ => None,
        }
    }

    /// 目的：测试用：往某个会话的流水追加事件（造「压过之后再重启」这类历史）。
    pub fn history_append(
        &mut self,
        sid: &str,
        events: &[serde_json::Value],
    ) -> Result<(), String> {
        self.history.append(sid, events)
    }

    /// 目的：该会话此刻的**身份块**（按当前提示词册与形态现渲染）：测试用它断言"它被告诉了什么"。
    pub fn single_identity(&self, sid: &str) -> Option<String> {
        match self.sessions.get(sid) {
            Some(Session::Single(s)) => Some(s.params().identity(&*self.prompt, s.tool_mode())),
            _ => None,
        }
    }
}

/// 继续被拦下时的提醒（单 agent 会话：该轮到用户发言）。
const NEED_USER: &str = "最后一条是 AI 发言：部分供应商不允许连续 AI 发言；请先发言再继续。";

/// 目的：会话列表按**树序**排：顶层保持后端给的时间倒序，每个会话后面**紧跟**它的子会话
///  （子会话内部同样按时间倒序），深度优先。
///  为什么必须在后端做：顺序与缩进是同一件事的两面——前端各排各的，就会出现子会话排在父会话上面。
pub(crate) fn tree_order(list: Vec<HistoryView>) -> Vec<HistoryView> {
    let mut kids: std::collections::BTreeMap<String, Vec<HistoryView>> =
        std::collections::BTreeMap::new();
    let mut roots: Vec<HistoryView> = Vec::new();
    for h in list {
        match h.parent.clone() {
            Some(p) => kids.entry(p).or_default().push(h),
            None => roots.push(h),
        }
    }
    let mut out: Vec<HistoryView> = Vec::new();
    for r in roots {
        let mut stack = vec![r];
        while let Some(cur) = stack.pop() {
            let mut mine = kids.remove(&cur.name).unwrap_or_default();
            mine.sort_by_key(|h| std::cmp::Reverse(h.ts));
            // 逆序压栈 → 弹出时按时间倒序（与顶层同一口径）。
            for k in mine.into_iter().rev() {
                stack.push(k);
            }
            out.push(cur);
        }
    }
    // 父会话已不在（被删）的子会话：如实列在最后，不能凭空消失。
    for (_, v) in kids {
        out.extend(v);
    }
    out
}

/// 会话是否「已经开过」（流水里有内容）：agent 名单与形态据此冻结。配置记录与回档记录不算内容。
fn session_started(events: &[serde_json::Value]) -> bool {
    events
        .iter()
        .any(|ev| match ev.get("type").and_then(|t| t.as_str()) {
            Some("config") | Some("rewind") => false,
            Some("transcript") => ev
                .get("lines")
                .and_then(|l| l.as_array())
                .map(|l| !l.is_empty())
                .unwrap_or(false),
            Some(_) => true,
            None => false,
        })
}

/// 工作形态 → 会话元信息里的标识。
fn mode_str(mode: WorkMode) -> &'static str {
    match mode {
        WorkMode::Single => "single",
        WorkMode::Collab => "collab",
        WorkMode::Proxy => "proxy",
    }
}

/// 当前时间戳（秒）。
fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 工作名即将来的落盘目录名：非空、不含路径分隔符与 Windows 非法字符、不是保留名。
fn validate_work_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("工作名不能为空".to_string());
    }
    if name != n {
        return Err("工作名首尾不能有空白".to_string());
    }
    if n == "." || n == ".." {
        return Err("工作名非法".to_string());
    }
    const BAD: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    if n.chars().any(|c| BAD.contains(&c) || c.is_control()) {
        return Err("工作名不能包含 \\ / : * ? \" < > | 或控制字符".to_string());
    }
    let upper = n.to_ascii_uppercase();
    let reserved_device = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved_device {
        return Err("工作名是系统保留名".to_string());
    }
    Ok(())
}
