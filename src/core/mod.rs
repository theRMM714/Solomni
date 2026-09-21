//! 核心层：定义抽象（ports）、编排业务（会话/引擎）、会话中心。
//! 分层纪律：本层不出现文件读、ureq、stdin/stdout——机制全部在 adapters，
//! 装配（new 适配器）只发生在 main 组合根。前端只见 Core 门面、会话句柄与 SessionEvent 流。

pub mod agents;
pub mod api;
pub mod collab;
pub mod collab_state;
pub mod engine;
pub mod envelope;
pub mod events;
pub mod exec;
pub mod fence;
pub mod history;
pub mod module;
pub mod packages;
pub mod patch;
pub mod ports;
pub mod prompt;
pub mod providers;
pub mod refs;
pub mod schema;
pub mod session;
pub mod systool;
pub mod workspace;

pub use events::{Pending, SessionEvent};
// 测试用同步入口的签名要它；生产路径的 Live 构造在 api.rs（那里直接引 events::Live）。
#[cfg(test)]
pub(crate) use events::Live;
pub use ports::{
    ChatGateway, HistoryStore, ModelCatalog, ModuleSource, PackageSource, PromptSource,
    SettingsStore, SysIo, ToolRunner, Workspace,
};

use crate::core::collab::CollabSession;
use crate::core::history::{AgentMeta, HistoryView, SessionMeta};
use crate::core::module::Module;
use crate::core::ports::Msg;
use crate::core::prompt::Prompts;
use crate::core::providers::{AppSettings, Channel, Settings};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// 前端唯一的会话标识 = 工作名（用户的命名，也是将来落盘目录名）。
pub type SessionId = String;

/// 会话实例：单 agent 会话或协作会话（本体自带端口，可跨线程移动）。
/// 两变体大小差得远（协作会话带整份讨论状态），装箱只换来一次间接寻址、
/// 却把"会话本体可直接移动"这个形状改掉——有意不装箱（见 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::large_enum_variant)]
pub enum Session {
    Single(session::AgentSession),
    Collab(CollabSession),
}

/// 协作推进阶段：由前端按 pending 决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollabStep {
    SetTask,
    ConfirmSlate,
    Begin,
    Answer,
}

/// 工作形态：单 agent（模块数不限）/ 协作（多 agent 分权协商）。
/// 形态只用于校验与界面标签：会话实现只有「单 agent」与「协作」两种。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkMode {
    Single,
    Collab,
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
}

/// 创建工作后的结果：最终实例名（重名已加尾号）与开场事件。
#[derive(Debug, Clone)]
pub struct WorkOpened {
    pub sid: SessionId,
    pub agents: Vec<String>,
    pub events: Vec<SessionEvent>,
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
    pub diagnoses: Vec<exec::Diagnosis>,
    /// 本机能不能承载**当前档位**（虚拟机档的前置条件；本机档恒为可）。
    /// 界面据此决定虚拟机档能不能选，并与「开始」/编辑的校验同源（`exec::tier_readiness`）。
    pub tier_ready: bool,
    /// 承载不了时缺什么（空 = 齐了）。
    pub tier_missing: Vec<String>,
    /// 被拒收的模块（原因如实）。
    pub rejected: Vec<String>,
    /// 被拒收的运行包（原因如实）。
    pub rejected_packages: Vec<String>,
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
    pub vm_requirements: Vec<crate::core::exec::VmRequirement>,
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

/// 一次单 agent 生成的**准备结果**（核心线程上只算到这里，模型调用在工作线程上）。
pub(crate) enum Prepared {
    /// 可以跑：会话已从核心表取出，由工作线程独占。
    Run {
        /// 装箱：这个变体比其它两个大得多（会话本体），而它本来就是**一次性移交**给工作线程的。
        session: Box<session::AgentSession>,
        /// 生成前要先给用户的事件（例如工具形态变更的提示）。
        prefix: Vec<SessionEvent>,
        llm: ports::LlmOpts,
    },
    /// 不用跑模型：直接把这批事件回给调用方（例如"末条是 AI 发言"）。
    Immediate(Vec<SessionEvent>),
    /// 不是单 agent 会话：交回调用方走它自己那条路（协作）。
    NotSingle,
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
}

/// 会话文件清单视图（前端 @ 菜单与「长路径缩写」用）：相对清单 + 真实根。
/// 根一律是 slash() 书写形式（/ 分隔、无扩展长度前缀）；拿不到根就给空串（前端原样显示，不猜）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilesView {
    pub work: Vec<String>,
    pub agents: Vec<FilesAgentView>,
    pub roots: FilesRootsView,
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

#[derive(serde::Deserialize)]
struct SuggestReply {
    agents: Vec<agents::RosterPick>,
}

/// 核心门面：持有注入的端口与会话中心；前端只经此操作。
/// 线程共享形态：组合根把它放进 Arc 加 Mutex（Web 多连接/多会话所需）。
pub struct Core {
    store: Arc<dyn SettingsStore + Send + Sync>,
    history: Arc<dyn HistoryStore + Send + Sync>,
    workspace: Arc<dyn Workspace + Send + Sync>,
    source: Arc<dyn ModuleSource + Send + Sync>,
    /// 运行包库来源（依赖文件夹的扫描事实；校验与诊断在 core）。
    packages: Arc<dyn PackageSource + Send + Sync>,
    /// 围栏授权释放（会话删除时请求一次；机制在适配层）。
    fence: Arc<dyn ports::FenceHost + Send + Sync>,
    gateway: Arc<dyn ChatGateway + Send + Sync>,
    catalog: Arc<dyn ModelCatalog + Send + Sync>,
    tools: Arc<dyn ToolRunner + Send + Sync>,
    /// 内置文件工具读写端口（策略在 core：寻址与越界校验）。
    io: Arc<dyn SysIo + Send + Sync>,
    /// 信封修复端口（手写信封不合法时的无歧义补救；默认真现在 adapters，可整体替换）。
    repair: Arc<dyn ports::EnvelopeRepair + Send + Sync>,
    log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    settings: Settings,
    prompts: Prompts,
    sessions: HashMap<SessionId, Session>,
    /// 正在生成的会话：对象被工作线程**取走**了，核心表里暂时没有它。
    /// 为什么取出而不是就地生成：生成要跑几十秒到几分钟，占着唯一的命令队列会让
    /// 读接口（历史列表、状态）与其它会话的命令全排在它后面——界面因此"假死"。
    /// 这一态只表示"不在表里是因为在生成"，不是"不存在"。
    running: std::collections::BTreeSet<SessionId>,
}

impl Core {
    /// 组合根专用：main 负责创建适配器并注入；core 不自建任何具体实现。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn SettingsStore + Send + Sync>,
        history: Arc<dyn HistoryStore + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        source: Arc<dyn ModuleSource + Send + Sync>,
        packages: Arc<dyn PackageSource + Send + Sync>,
        fence: Arc<dyn ports::FenceHost + Send + Sync>,
        gateway: Arc<dyn ChatGateway + Send + Sync>,
        catalog: Arc<dyn ModelCatalog + Send + Sync>,
        tools: Arc<dyn ToolRunner + Send + Sync>,
        io: Arc<dyn SysIo + Send + Sync>,
        repair: Arc<dyn ports::EnvelopeRepair + Send + Sync>,
        prompt_source: Box<dyn PromptSource>,
        log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    ) -> Result<Core, String> {
        let log_for_core = Arc::clone(&log);
        let outcome = (|| -> Result<Core, String> {
            let settings = store.load()?;
            let prompts = prompt_source.load()?;
            Ok(Core {
                store,
                history,
                workspace,
                source,
                packages,
                fence,
                gateway,
                catalog,
                tools,
                io,
                repair,
                log: log_for_core,
                settings,
                prompts,
                sessions: HashMap::new(),
                running: std::collections::BTreeSet::new(),
            })
        })();
        if let Err(e) = &outcome {
            log.error("core::new", &format!("装配失败：{}", e)); // 仅错误时借用，不与闭包 move 冲突
        }
        outcome
    }

    /// 日志端口句柄：入站手柄（core::api）与组合根共用同一份事实记录。
    pub fn log_handle(&self) -> Arc<dyn crate::core::ports::Log + Send + Sync> {
        Arc::clone(&self.log)
    }

    /// 生成期间"会话不在表里"的三种进入点共用这一句（错误文案要一致，别处不再各写一份）。
    fn running_refusal(sid: &str) -> String {
        format!("会话 {} 正在生成中：先「停止」或等它结束，再做这一步", sid)
    }

    /// 把单 agent 会话**交给工作线程**（核心表里留"生成中"）。
    /// 取出的窗口内，核心队列是空的——读接口与其它会话的命令因此照常。
    pub(crate) fn take_single(&mut self, sid: &str) -> Result<session::AgentSession, String> {
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

    /// 把协作会话**交给工作线程**（核心表里留"生成中"）。
    /// 会话还没装进内存时先从落盘重建：协作的"继续"可能先于"打开"到达。
    pub(crate) fn take_collab(&mut self, sid: &str) -> Result<CollabSession, String> {
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

    /// 协作生成结束**交回**：重新插入 + 转录落盘 + 解除"生成中"。
    pub(crate) fn put_collab(&mut self, sid: &str, c: CollabSession, events: &[SessionEvent]) {
        self.running.remove(sid);
        self.sessions.insert(sid.to_string(), Session::Collab(c));
        let mut ev = events.to_vec();
        self.record_events(sid, &mut ev);
    }

    /// 生成结束**交回**：重新插入 + 转录落盘 + 解除"生成中"。
    /// 所有状态变更仍只发生在核心线程上（工作线程只跑生成，不碰核心状态）。
    pub(crate) fn put_single(
        &mut self,
        sid: &str,
        s: session::AgentSession,
        events: &[SessionEvent],
    ) {
        self.running.remove(sid);
        self.sessions.insert(sid.to_string(), Session::Single(s));
        let mut ev = events.to_vec();
        self.record_events(sid, &mut ev);
    }

    /// 生成线程崩溃：会话对象随线程一起没了，但**转录在盘上**。
    /// 只解除"生成中"，下次访问按落盘转录重建——绝不把会话卡在"生成中"。
    pub(crate) fn abort_running(&mut self, sid: &str) {
        self.running.remove(sid);
    }

    /// 清单即事实：每次调用重扫（策略在 core，机制在 ModuleSource）。
    pub fn scan(&self) -> module::Roster {
        self.source.scan()
    }

    /// 运行能力报告：模块声明的能力、包库里的可用版本、缺失清单与虚拟机档诊断。
    /// 「清单即事实」：每次调用重扫模块清单与包库；本机档不装载运行包，missing 只作事实呈现。
    pub fn runtime_report(&self, tier: exec::Tier) -> RuntimeReport {
        let roster = self.source.scan();
        let lib = self.packages.scan();
        let spec = exec::ExecSpec {
            tier,
            ..exec::ExecSpec::default()
        };
        let diagnoses = if tier == exec::Tier::Vm {
            exec::vm_diagnoses(&roster.modules, &lib, &spec)
        } else {
            Vec::new()
        };
        let readiness = exec::tier_readiness(&spec, self.qemu_path());
        RuntimeReport {
            tier: tier.as_str().to_string(),
            declared: exec::declared(&roster.modules),
            available: lib.capability_versions(),
            missing: exec::absent(&roster.modules, &lib),
            diagnoses,
            tier_ready: readiness.ready(),
            tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
            rejected: roster.rejected.clone(),
            rejected_packages: lib.rejected.clone(),
        }
    }

    /// 本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
    fn unavailable_modules(
        &self,
        spec: &exec::ExecSpec,
        modules: &[Module],
    ) -> BTreeMap<String, Vec<String>> {
        exec::unavailable(spec, modules, &self.packages.scan())
    }

    /// 配置视图：把「能改什么、现在是什么、缺什么」如实给出（每次读取都重扫模块清单与包库）。
    pub fn session_config(&self, sid: &str) -> Result<SessionConfig, String> {
        let (meta, events) = self.history_open(sid)?;
        let tier = meta.exec.tier;
        // 虚拟机档的承载探针：用用户填的基础根（若有），否则问"裸虚拟机档"能不能成立。
        let vm_probe = exec::ExecSpec {
            tier: exec::Tier::Vm,
            base: meta.exec.base.clone(),
            ..exec::ExecSpec::default()
        };
        Ok(SessionConfig {
            sid: meta.name.clone(),
            mode: meta.mode.clone(),
            started: session_started(&events),
            agents: meta
                .agents
                .iter()
                .map(|a| ConfigAgent {
                    name: a.name.clone(),
                    modules: a.modules.clone(),
                    model: a.model.clone().unwrap_or_default(),
                })
                .collect(),
            tier: tier.as_str().to_string(),
            base: meta.exec.base.clone(),
            net: meta.exec.net,
            pins: meta.exec.pins.clone(),
            runtime: self.runtime_report(tier),
            runtimes_dir: workspace::slash(&self.packages.dir()),
            tier_ready: exec::tier_readiness(&meta.exec, self.qemu_path()).ready(),
            tier_missing: exec::tier_readiness(&meta.exec, self.qemu_path())
                .missing()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            // 虚拟机档能不能选**与当前档位无关**：本机档会话也要如实告诉用户 vm 现在不可用（界面据此禁用）。
            // 逐项清单一起给出：界面照抄"缺哪几项、每项怎么补"，不自己编话。
            vm_available: exec::tier_readiness(&vm_probe, self.qemu_path()).ready(),
            vm_unavailable_reason: exec::tier_refusal(&vm_probe, self.qemu_path())
                .unwrap_or_default(),
            vm_requirements: exec::vm_requirements(&exec::VmInputs {
                base: vm_probe.base.as_deref(),
                qemu: self.qemu_path(),
            }),
        })
    }

    /// 编辑提交：校验 → 写回 meta.yaml（名单与选型的唯一真相）→ 追加一条旁路配置记录 → 丢掉内存会话。
    /// 生效点：下一次访问按新配置从转录重建会话对象（所以改完不必重开会话）。
    /// 冻结：流水里有内容（会话已经开过）时，agent 名单与形态不可改——换人请新建会话。
    pub fn edit_session(&mut self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, events) = self.history_open(sid)?;
        match meta.mode.as_str() {
            "single" | "collab" => {}
            other => return Err(format!("未知会话形态：{}（只认 single / collab）", other)),
        }
        if session_started(&events) {
            let old: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
            let new: Vec<String> = edit.agents.iter().map(|a| a.name.clone()).collect();
            if old != new {
                return Err(
                    "这轮会话已经开过：agent 名单与形态冻结（要换人请新建会话）".to_string()
                );
            }
        }
        // 校验：模块与模型真实存在；同一模块不得同属两个 agent（沙箱与发言归属会歧义）。
        let roster = self.scan();
        let mut seen: Vec<String> = Vec::new();
        let mut metas: Vec<AgentMeta> = Vec::new();
        for a in &edit.agents {
            agents::validate_name(&a.name)?;
            if a.modules.is_empty() {
                return Err(format!("agent {} 至少要有一个模块", a.name));
            }
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
                if seen.iter().any(|x| x == id) {
                    return Err(format!(
                        "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                        id
                    ));
                }
                seen.push(id.clone());
            }
            if !a.model.is_empty() {
                if !self.settings.models.contains_key(&a.model) {
                    return Err(format!("无此模型：{}", a.model));
                }
                self.settings.resolve(&a.model)?;
            }
            metas.push(AgentMeta {
                name: a.name.clone(),
                transient: !self.settings.agents.contains_key(&a.name),
                modules: a.modules.clone(),
                model: if a.model.is_empty() {
                    None
                } else {
                    Some(a.model.clone())
                },
            });
        }
        if metas.is_empty() {
            return Err("至少要有一个 agent".to_string());
        }
        if meta.mode == "single" && metas.len() != 1 {
            return Err("单 agent 形态只接受一个 agent（模块数不限）".to_string());
        }
        // 档位：与「开始」同一把尺子——虚拟机档的选型不成立（多版本未定版 / 定版不存在 / 路径冲突）如实拒绝。
        let tier = match edit.tier.as_str() {
            "host" => exec::Tier::Host,
            "vm" => exec::Tier::Vm,
            other => return Err(format!("未知执行档位：{}（只认 host / vm）", other)),
        };
        let spec = exec::ExecSpec {
            tier,
            base: edit.base.clone(),
            pins: edit.pins.clone(),
            net: edit.net,
        };
        // 承载校验：前置条件不具备时**不允许改入虚拟机档**（用户环境问题，不是选型问题）。
        // 界面上的"能不能选"由 SessionConfig 的 tier_ready 说同一件事，两处不会各说各话。
        // 已经在虚拟机档上的会话只校验**它自己那几项**（基础根等）：改模块、改模型、定版、开网络都不该被拦住——
        // 一条已存在的会话连改都不让改，是拿用户自己的记录当人质。
        let staying_vm = meta.exec.tier == exec::Tier::Vm && tier == exec::Tier::Vm;
        if staying_vm {
            // 留在 vm 档：只校验用户这次填的基础根（填错路径就是填错路径），
            // 不拿"本机能不能提供 vm 档"去拦一条已经存在的会话。
            let base_item = exec::tier_readiness(&spec, self.qemu_path())
                .requirements
                .into_iter()
                .find(|r| r.id == "base");
            if let Some(item) = base_item {
                if !item.met {
                    return Err(format!("{}：{}", item.detail, item.how));
                }
            }
        } else if let Some(why) = exec::tier_refusal(&spec, self.qemu_path()) {
            return Err(why);
        }
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| seen.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        let plan = exec::plan(&spec, &session_modules, &self.packages.scan())
            .map_err(|diags| exec::diagnose_text(&diags))?;
        self.log.info(
            "core::edit_session",
            &format!("sid={}；{}", sid, exec::plan_summary(&plan)),
        );

        let mut new_meta = meta.clone();
        new_meta.modules = metas.iter().flat_map(|a| a.modules.clone()).collect();
        new_meta.agents = metas;
        new_meta.exec = spec;
        self.history.save_meta(&new_meta)?;
        self.record_config(sid, &new_meta);
        // 内存里那份是按旧配置装的：丢掉它，下一次访问按新配置从转录重建（转录即状态，不丢内容）。
        self.sessions.remove(sid);
        Ok(())
    }

    /// 追加一条旁路配置记录：只作呈现与审计（不进模型上下文，回放与状态派生都跳过它）。
    fn record_config(&self, sid: &str, meta: &SessionMeta) {
        let ev = serde_json::json!({
            "type": "config",
            "ts": now_ts(),
            "mode": meta.mode,
            "tier": meta.exec.tier.as_str(),
            "base": meta.exec.base,
            "net": meta.exec.net,
            "pins": meta.exec.pins,
            "agents": meta.agents.iter().map(|a| serde_json::json!({
                "name": a.name,
                "modules": a.modules,
                "model": a.model,
            })).collect::<Vec<_>>(),
        });
        if let Err(e) = self.history.append(sid, &[ev]) {
            self.log
                .warn("core::record_config", &format!("配置记录落盘失败：{}", e));
        }
    }

    // ---- 登记处：供应商（密钥只在此层进出；前端只见 id 与端点） ----

    /// 结构化供应商视图（不含密钥；Web 用）。
    pub fn provider_views(&self) -> Vec<providers::ProviderView> {
        self.settings.provider_views()
    }

    /// 新建/更新供应商。更新时 api_key 留空 = 保留原密钥（界面从不回显密钥）。
    pub fn provider_upsert(
        &mut self,
        id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<(), String> {
        if id.is_empty() || base_url.is_empty() {
            return Err("id / base_url 不能为空".to_string());
        }
        let key = if api_key.is_empty() {
            self.settings
                .providers
                .get(id)
                .map(|p| p.api_key.clone())
                .ok_or_else(|| "api_key 不能为空".to_string())?
        } else {
            api_key.to_string()
        };
        self.settings.providers.insert(
            id.to_string(),
            providers::Provider {
                base_url: base_url.to_string(),
                api_key: key,
            },
        );
        self.save_settings("core::provider_upsert")
    }

    /// 删除供应商；仍被模型引用时拒绝（不静默级联删除）。
    pub fn provider_remove(&mut self, id: &str) -> Result<bool, String> {
        let referenced: Vec<String> = self
            .settings
            .models
            .iter()
            .filter(|(_, m)| m.provider == id)
            .map(|(mid, _)| mid.clone())
            .collect();
        if !referenced.is_empty() {
            return Err(format!(
                "供应商 {} 仍被模型引用：{}；请先删除这些模型",
                id,
                referenced.join("、")
            ));
        }
        let removed = self.settings.providers.remove(id).is_some();
        if removed {
            self.save_settings("core::provider_remove")?;
        }
        Ok(removed)
    }

    // ---- 登记处：模型（独立实体，引用供应商；两者分开保存） ----

    /// 结构化模型视图（Web 用）。
    pub fn model_views(&self) -> Vec<providers::ModelView> {
        self.settings.model_views()
    }

    /// 核心 AI 默认模型 id。
    pub fn core_model(&self) -> Option<String> {
        self.settings.core.clone()
    }

    /// 实测一条通道支不支持原生工具调用，并把**结论写回登记处**（只写确定的结论）：
    /// 支持 → `tools: native`；明确不支持 → `tools: envelope`；无法判定 → 不改，只把事实报回去。
    /// 事实由适配层实测（两条最小请求对比），core 只做"要不要落盘"这一层策略。
    pub fn probe_model_tools(&mut self, id: &str) -> Result<providers::ProbeOutcome, String> {
        let channel = self.settings.resolve(id)?;
        let outcome = self.gateway.probe_tools(&channel)?;
        let want = match &outcome {
            providers::ProbeOutcome::Supported { .. } => Some(providers::ToolMode::Native),
            providers::ProbeOutcome::Unsupported { .. } => Some(providers::ToolMode::Envelope),
            providers::ProbeOutcome::Unknown { .. } => None,
        };
        if let Some(mode) = want {
            if let Some(m) = self.settings.models.get_mut(id) {
                if m.tools != mode {
                    m.tools = mode;
                    self.save_settings("core::probe_model_tools")?;
                }
            }
        }
        Ok(outcome)
    }

    /// 实测一条通道的**回放形状**（工具调用历史怎么发回去才收）：解析 id → 交给适配层实测。
    /// 只报事实、**不写登记处**——采不采用由人定（与 probe_model_tools 的写回策略不同）。
    pub fn probe_replay_shape(&self, id: &str) -> Result<providers::ReplayReport, String> {
        let channel = self.settings.resolve(id)?;
        self.gateway.probe_replay(&channel)
    }

    pub fn model_upsert(
        &mut self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
    ) -> Result<(), String> {
        if id.is_empty() || name.is_empty() || api_model.is_empty() || provider.is_empty() {
            return Err("id / name / api_model / provider 均不能为空".to_string());
        }
        if !self.settings.providers.contains_key(provider) {
            return Err(format!("无此供应商：{}", provider));
        }
        // 工具调用形态：编辑时**保留原值**（登记表单暂不带这个字段，不能因为没带就重置成缺省），
        // 新建缺省 envelope（任何供应商都能用的手写信封）。
        let tools = self
            .settings
            .models
            .get(id)
            .map(|m| m.tools)
            .unwrap_or_default();
        self.settings.models.insert(
            id.to_string(),
            providers::ModelEntry {
                name: name.to_string(),
                api_model: api_model.to_string(),
                provider: provider.to_string(),
                note: note.to_string(),
                tools,
            },
        );
        self.save_settings("core::model_upsert")
    }

    /// 删除模型；是核心默认模型时拒绝（先改默认再删）。
    pub fn model_remove(&mut self, id: &str) -> Result<bool, String> {
        if self.settings.core.as_deref() == Some(id) {
            return Err(format!(
                "{} 是核心默认模型；请先把核心默认模型改成别的再删",
                id
            ));
        }
        let removed = self.settings.models.remove(id).is_some();
        if removed {
            self.save_settings("core::model_remove")?;
        }
        Ok(removed)
    }

    pub fn core_set_model(&mut self, id: &str) -> Result<bool, String> {
        if !self.settings.models.contains_key(id) {
            return Ok(false);
        }
        self.settings.core = Some(id.to_string());
        self.save_settings("core::core_set_model")?;
        Ok(true)
    }

    // ---- agent（用户配置的具名能力组合） ----

    pub fn agent_views(&self) -> Vec<agents::AgentView> {
        self.settings
            .agents
            .iter()
            .map(|(name, a)| agents::AgentView {
                name: name.clone(),
                modules: a.modules.clone(),
                model: a.model.clone(),
                note: a.note.clone(),
            })
            .collect()
    }

    /// 新建/覆盖一个 agent（校验模块与模型都真实存在；不静默）。
    pub fn agent_upsert(
        &mut self,
        name: &str,
        module_ids: &[String],
        model: &str,
        note: &str,
    ) -> Result<(), String> {
        agents::validate_name(name)?;
        if module_ids.is_empty() {
            return Err("agent 至少要有一个模块".to_string());
        }
        let roster = self.scan();
        for id in module_ids {
            if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                return Err(format!("无此模块：{}", id));
            }
        }
        if !model.is_empty() && !self.settings.models.contains_key(model) {
            return Err(format!("无此模型：{}", model));
        }
        self.settings.agents.insert(
            name.to_string(),
            agents::Agent {
                modules: module_ids.to_vec(),
                model: if model.is_empty() {
                    None
                } else {
                    Some(model.to_string())
                },
                note: note.to_string(),
            },
        );
        self.save_settings("core::agent_upsert")
    }

    pub fn agent_remove(&mut self, name: &str) -> Result<bool, String> {
        let removed = self.settings.agents.remove(name).is_some();
        if removed {
            self.save_settings("core::agent_remove")?;
        }
        Ok(removed)
    }

    /// 基本设置。
    pub fn app_settings(&self) -> AppSettings {
        self.settings.app.clone()
    }

    /// 用户显式授权的只读根（`settings.yaml` 的 `fence_read`）。
    /// 策略层只带事实：哪些目录只读可达由用户定，只读位怎么落由适配层定。
    /// 空 = 一个都不放行（默认不动本机任何权限项）。
    /// 本次模型调用的通道参数：**预算与"能不能流式"都取全局设置**（讨论、执行、验收、单 agent 共用一份）。
    /// `want_stream` 是调用方这一次的意愿（呈现层按回包形状给）：**设置是上限，调用方可以在本次放弃流式**；
    /// 设置关掉时一律非流式。两处各判一次迟早会打架，所以判据只在这里。
    pub(crate) fn llm_opts(&self, want_stream: bool) -> crate::core::ports::LlmOpts {
        crate::core::ports::LlmOpts {
            stream: self.settings.app.streaming && want_stream,
            timeout_secs: self.settings.app.llm_timeout_secs,
        }
    }

    /// 设置里登记的 QEMU 可执行文件路径（默认空 = 兜底看 PATH）。产品不自带、不下载 QEMU。
    fn qemu_path(&self) -> Option<&str> {
        let p = self.settings.app.qemu_path.trim();
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    }

    fn fence_read_roots(&self) -> Vec<std::path::PathBuf> {
        self.settings
            .app
            .fence_read
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect()
    }

    pub fn set_app_settings(&mut self, app: AppSettings) -> Result<(), String> {
        self.settings.app = app;
        self.save_settings("core::set_app_settings")
    }

    /// 用登记处已存的供应商去拉取其可用模型名（发现机制在适配层）。
    pub fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let provider = self
            .settings
            .providers
            .get(provider_id)
            .ok_or_else(|| format!("无此供应商：{}", provider_id))?;
        let outcome = self.catalog.list_models(provider);
        match &outcome {
            Ok(models) => self.log.info(
                "core::discover_models",
                &format!("供应商 {} 拉取模型 {} 个", provider_id, models.len()),
            ),
            Err(e) => self.log.error(
                "core::discover_models",
                &format!("供应商 {} 拉取模型失败：{}", provider_id, e),
            ),
        }
        outcome
    }

    fn save_settings(&self, at: &str) -> Result<(), String> {
        let r = self.store.save(&self.settings);
        if let Err(e) = &r {
            self.log.error(at, &format!("登记处持久化失败：{}", e));
        }
        r
    }

    // ---- 会话中心（前端只持 id） ----

    /// 会话列表视图（进行中的工作）。形态取落盘 meta（单一真相，不在内存里留影子状态）。
    pub fn session_views(&self, history: &[HistoryView]) -> Vec<SessionView> {
        // 在表里的会话 + **正在生成的会话**（后者对象在工作线程上，但它确实存在、也确实在跑）。
        // 漏掉它们会让界面以为会话不见了。
        let running: Vec<(String, bool)> = self
            .running
            .iter()
            .map(|sid| (sid.clone(), false))
            .collect();
        let listed: Vec<(String, bool)> = self
            .sessions
            .iter()
            .map(|(sid, s)| {
                let done = match s {
                    Session::Collab(c) => c.is_done(),
                    Session::Single(_) => false,
                };
                (sid.clone(), done)
            })
            .chain(running)
            .collect();
        listed
            .into_iter()
            .map(|(sid, done)| {
                let entry = history.iter().find(|h| h.name == sid);
                let mode = entry.map(|h| h.mode.clone()).unwrap_or_default();
                // 记的档位来自落盘 meta（权威）：环境后来变了也要如实提示——**不拦打开**（记录是用户的）。
                let exec = entry.map(|h| h.exec.clone()).unwrap_or_default();
                let readiness = exec::tier_readiness(&exec, self.qemu_path());
                SessionView {
                    sid,
                    mode,
                    done,
                    tier: exec.tier.as_str().to_string(),
                    tier_ready: readiness.ready(),
                    tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
                }
            })
            .collect()
    }

    pub fn session_exists(&self, name: &str) -> bool {
        self.sessions.contains_key(name)
    }

    /// 创建工作：形态 + 参与的 agent（+ 协作需求）→ 建出会话、落盘身份、备好工作区。
    /// 一切选择来自用户；核心只做校验与机械装配，不替用户选。
    pub fn create_work(&mut self, spec: WorkSpec) -> Result<WorkOpened, String> {
        validate_work_name(&spec.name)?;
        if self.sessions.contains_key(&spec.name) || self.history.load(&spec.name).is_ok() {
            return Err(format!("工作名已存在：{}", spec.name));
        }
        // 代拟路径（协作、未给 agent）允许先空着，由核心按需求拟名单；其余形态必须有 agent。
        if spec.agents.is_empty() && !spec.delegate {
            return Err("至少要有一个 agent".to_string());
        }
        let roster = self.scan();
        // 校验 agent：名字合法、模块与模型真实存在；同一模块不得同时属于两个 agent（沙箱会歧义）
        let mut seen_modules: Vec<String> = Vec::new();
        for a in &spec.agents {
            agents::validate_name(&a.name)?;
            if a.modules.is_empty() {
                return Err(format!("agent {} 至少要有一个模块", a.name));
            }
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
                if seen_modules.iter().any(|x| x == id) {
                    return Err(format!(
                        "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                        id
                    ));
                }
                seen_modules.push(id.clone());
            }
            if let Some(mid) = &a.model {
                if !self.settings.models.contains_key(mid) {
                    return Err(format!("无此模型：{}", mid));
                }
                self.settings.resolve(mid)?;
            }
        }
        // 形态约束（每个 agent ≥1 个模块已在上面的循环里校验）
        match spec.mode {
            WorkMode::Single => {
                if spec.agents.len() != 1 {
                    return Err("单 agent 形态只接受一个 agent（模块数不限）".to_string());
                }
            }
            WorkMode::Collab => {
                if spec.task.as_deref().unwrap_or("").trim().is_empty() {
                    return Err("协作模式必须填写本次需求".to_string());
                }
            }
        }
        // 同工作内重名 → 尾号（用户没改名时的兜底）
        let mut taken: Vec<String> = Vec::new();
        let mut metas: Vec<AgentMeta> = Vec::new();
        for a in &spec.agents {
            let name = agents::unique_instance_name(&a.name, &taken);
            taken.push(name.clone());
            metas.push(AgentMeta {
                name,
                transient: a.transient,
                modules: a.modules.clone(),
                model: a.model.clone(),
            });
        }
        // 模块扁平清单（展示用；顺序按 agent 名单展开）
        let module_ids: Vec<String> = metas.iter().flat_map(|m| m.modules.clone()).collect();
        // 代拟：给了 agent 就不再代拟（名单已经有了，不让核心盖掉用户的选择）。
        let delegate = spec.delegate && metas.is_empty();

        let name = spec.name.clone();
        let agent_names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let meta = SessionMeta {
            name: name.clone(),
            mode: mode_str(spec.mode).to_string(),
            delegate,
            modules: module_ids.clone(),
            task: spec.task.clone(),
            ts: now_ts(),
            agents: metas.clone(),
            exec: exec::ExecSpec {
                tier: self.settings.app.tier,
                ..exec::ExecSpec::default()
            },
        };
        // 承载校验：默认档位的前置条件不具备时**不允许创建虚拟机档会话**（用户环境问题，不是选型问题）。
        // 必须在建工作区之前收口——拒绝就该什么都不留下。
        if let Some(why) = exec::tier_refusal(&meta.exec, self.qemu_path()) {
            return Err(why);
        }
        // 工作区：work + 各 agent 沙箱（失败即失败，不假装已建）。代拟确认名单时再补建。
        self.workspace.prepare(&name, &agent_names)?;
        let sandboxes = self.sandboxes(&meta, &roster)?;
        // 扫描事实如实埋点：本会话用到的模块里，哪些声明的运行包不在包库（缺包不等于崩溃，工具按档位不可用）。
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| module_ids.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        for (id, caps) in exec::absent(&session_modules, &self.packages.scan()) {
            self.log.warn(
                "core::create_work",
                &format!("模块 {} 声明的运行包不在包库：{}", id, caps.join("、")),
            );
        }
        // 执行选型的完整性检查（「开始」即冻结）：虚拟机档的选型不成立（多版本未定版 / 定版不存在 /
        // 路径冲突）如实拒绝；只是缺包的照常开始——那是该模块的工具不可用（降级而非崩溃）。装配阶段按同一份计划取包。
        let plan = exec::plan(&meta.exec, &session_modules, &self.packages.scan())
            .map_err(|diags| exec::diagnose_text(&diags))?;
        self.log
            .info("core::create_work", &exec::plan_summary(&plan));

        let (session, mut events) = match spec.mode {
            // 单 agent（模块数不限）。
            WorkMode::Single => {
                let a = metas.first().ok_or("至少要有一个 agent")?;
                let sb = sandboxes
                    .for_agent(&a.name)
                    .cloned()
                    .ok_or_else(|| format!("缺少 agent {} 的沙箱", a.name))?;
                let chosen: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| {
                        roster
                            .modules
                            .iter()
                            .find(|m| &m.manifest.id == id)
                            .cloned()
                    })
                    .collect();
                let channel = self.channel_of(a.model.as_deref());
                let unavailable = self.unavailable_modules(&meta.exec, &chosen);
                let (s, opened) =
                    self.build_single(a, &chosen, channel, &sb, unavailable, meta.exec.net);
                (Session::Single(s), opened)
            }
            WorkMode::Collab => {
                let task = spec.task.as_deref().unwrap_or("").trim().to_string();
                let mut cs = CollabSession::start(
                    Arc::clone(&self.gateway),
                    Arc::clone(&self.source),
                    self.settings.clone(),
                    self.prompts.clone(),
                    Arc::clone(&self.tools),
                    Arc::clone(&self.io),
                    Arc::clone(&self.repair),
                    Arc::clone(&self.log),
                    Arc::clone(&self.packages),
                    meta.exec.clone(),
                    metas.clone(),
                    delegate,
                    sandboxes.clone(),
                )?;
                let mut out = Vec::new();
                cs.set_task(&task, &mut |e| out.push(e));
                (Session::Collab(cs), out)
            }
        };
        // 会话身份落盘：失败即失败（名字即目录，这是承诺，不假装已保存）。
        self.history.create(&meta)?;
        self.sessions.insert(name.clone(), session);
        self.record_events(&name, &mut events);
        Ok(WorkOpened {
            sid: name,
            agents: agent_names,
            events,
        })
    }

    /// 界面投喂：把文件写进本次工作的 work/。
    /// 返回 Ok(false) = 同名文件已存在且未选择覆盖（交前端让用户决定：覆盖/改名/取消）。
    pub fn work_upload(
        &mut self,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        // 策略在 core：先净化文件名，再交给工作区端口（机制只看已净化的名字）。
        let name = workspace::safe_file_name(name)?;
        if !self.sessions.contains_key(sid) && self.history.load(sid).is_err() {
            return Err(format!("无此会话：{}", sid));
        }
        if self.workspace.work_has(sid, &name) && !overwrite {
            return Ok(false);
        }
        self.workspace.write_work(sid, &name, bytes)?;
        Ok(true)
    }

    /// 本工作可引用的文件清单 + 真实根（前端 @ 菜单与「长路径缩写」用）。
    /// 名单取自会话 meta（活动会话与历史会话都以 meta.agents 为权威）；列目录的机制在 Workspace 端口，
    /// 根取自沙箱（与文件清单同源）：agents 与 roots.agents 同序同名。
    pub fn files_view(&self, sid: &str) -> Result<FilesView, String> {
        let (meta, _) = self
            .history
            .load(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let files = self.workspace.list(sid, &names)?;
        let roster = self.scan();
        let sandboxes = self.sandboxes(&meta, &roster)?;
        let mut agents: Vec<FilesAgentView> = Vec::new();
        let mut agent_roots: Vec<FilesAgentRootView> = Vec::new();
        for a in &meta.agents {
            agents.push(FilesAgentView {
                name: a.name.clone(),
                files: files.agents.get(&a.name).cloned().unwrap_or_default(),
            });
            agent_roots.push(FilesAgentRootView {
                name: a.name.clone(),
                root: sandboxes
                    .for_agent(&a.name)
                    .map(|s| workspace::slash(&s.private))
                    .unwrap_or_default(),
            });
        }
        Ok(FilesView {
            work: files.work,
            agents,
            roots: FilesRootsView {
                work: workspace::slash(&sandboxes.shared),
                agents: agent_roots,
            },
        })
    }

    /// 核心按用户选择解析模型通道；缺省用核心默认；都缺 = None（网关回落演示并告知）。
    fn channel_of(&self, model: Option<&str>) -> Option<Channel> {
        match model {
            Some(id) => self.settings.resolve(id).ok(),
            None => self.settings.core_channel(),
        }
    }

    /// 组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    /// 权限策略在此收口：模块目录只对其所属 agent 可达（同一模块不会同属两个 agent，创建时已校验）。
    /// 每个 agent 的沙箱按 meta.agents 建；共享区根任何时候都有（代拟还没名单时也有，@ 改写要用）。
    fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &module::Roster,
    ) -> Result<workspace::Sandboxes, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let roots = self.workspace.roots(&meta.name, &names)?;
        let mut list: Vec<workspace::Sandbox> = Vec::new();
        for a in &meta.agents {
            let private = roots
                .agents
                .get(&a.name)
                .cloned()
                .ok_or_else(|| format!("工作区没有给出 agent {} 的沙箱路径", a.name))?;
            let mut modules = BTreeMap::new();
            for id in &a.modules {
                if let Some(m) = roster.modules.iter().find(|m| &m.manifest.id == id) {
                    modules.insert(id.clone(), m.root.clone());
                }
            }
            list.push(workspace::Sandbox {
                work_name: meta.name.clone(),
                agent: a.name.clone(),
                shared: roots.shared.clone(),
                private,
                modules,
                texts: self.prompts.core.tool_texts.clone(),
                builtin_tools: self.prompts.core.builtin_tools.clone(),
            });
        }
        Ok(workspace::Sandboxes {
            shared: roots.shared,
            list,
        })
    }

    /// 工具环境：内置文件工具永远可用；外部工具按模块分组放行（模块 id → 目录 + 工具表）。
    /// unavailable：本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行，并如实报缺哪个能力）。
    /// net：会话是否放行出站网络（exec 段；默认否），随围栏交给机制层。
    fn tools_env(
        &self,
        modules: &[Module],
        sb: &workspace::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        mode: providers::ToolMode,
    ) -> engine::MemberTools {
        engine::MemberTools {
            mode,
            modules: engine::tool_table(modules),
            observations: systool::Observations::default(),
            repair: Arc::clone(&self.repair),
            log: Arc::clone(&self.log),
            runner: Arc::clone(&self.tools),
            sandbox: sb.clone(),
            io: Arc::clone(&self.io),
            unavailable,
            // 围栏：可达范围 + 断网 + 环境白名单的落点，全部由该 agent 的沙箱派生（机制在 adapters）；
            // 只读根来自用户显式授权（`fence_read`），默认空。
            fence: crate::core::fence::FenceSpec::from_sandbox(sb, net)
                .with_read_only(self.fence_read_roots()),
            // 从零开始；按落盘转录重建时由调用方按转录里的最大值续号（见 rebuild_session）。
            reply_seq: 0,
            // 流式与预算取全局设置（与协作会话共用同一份；这里不预设"本次要不要流式"，由调用方给）。
            llm: self.llm_opts(true),
        }
    }

    /// 装配单 agent 会话（不插入会话中心；插入与落盘由 create_work 统一做）。
    fn build_single(
        &self,
        a: &AgentMeta,
        modules: &[Module],
        channel: Option<Channel>,
        sb: &workspace::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
    ) -> (session::AgentSession, Vec<SessionEvent>) {
        let (chat, note) = self.gateway.member_channel(channel.as_ref(), &a.name);
        // 形态按登记处解析；没有真实通道（演示回落）只能是手写信封——演示通道不会原生调用。
        let mode = if channel.is_some() {
            self.settings.tool_mode_for(a.model.as_deref())
        } else {
            providers::ToolMode::Envelope
        };
        self.log.info(
            "core::build_single",
            &format!(
                "单 agent 工作：agent {}，模块 {}，模型 {}",
                a.name,
                a.modules.join("+"),
                channel
                    .as_ref()
                    .map(|c| c.model.as_str())
                    .unwrap_or("无（演示）")
            ),
        );
        let system = module::agent_system(
            &self.prompts,
            &a.name,
            modules,
            &systool::guide(&self.prompts, sb),
            mode,
        );
        let tools = self.tools_env(modules, sb, unavailable, net, mode);
        let roots = crate::core::refs::RefRoots {
            work: sb.shared.clone(),
            private: Some(sb.private.clone()),
        };
        let s = session::AgentSession::new(
            &a.name,
            system,
            chat,
            note,
            Some(tools),
            self.prompts.core.refs.clone(),
            roots,
            self.prompts.core.tool_texts.clone(),
        );
        let opened = s.open();
        (s, opened)
    }

    // ---- 会话历史 ----

    /// 历史列表（读盘失败如实记日志，返回空表）。
    pub fn history_list(&self) -> Vec<HistoryView> {
        match self.history.list() {
            Ok(v) => v,
            Err(e) => {
                self.log
                    .error("core::history_list", &format!("会话列表失败：{}", e));
                Vec::new()
            }
        }
    }

    /// 打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
    /// 回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
    pub fn history_open(
        &self,
        name: &str,
    ) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let (meta, events) = self.history.load(name)?;
        Ok((meta, truncate_events(&events)))
    }

    /// 删除会话（= 删目录）。内存中的同名会话一并移除，避免内存与磁盘不一致。
    /// 删之前先请适配层撤销该会话各 agent 的围栏授权：痕迹与会话同生共死，不随会话数量堆积。
    pub fn history_delete(&mut self, name: &str) -> Result<bool, String> {
        if self.running.contains(name) {
            return Err(Self::running_refusal(name));
        }
        if let Ok((meta, _)) = self.history.load(name) {
            let roster = self.source.scan();
            match self.sandboxes(&meta, &roster) {
                Ok(sandboxes) => {
                    for sb in &sandboxes.list {
                        // 撤销要覆盖同一次授权写下的全部条目：读写根 + 用户授权的只读根。
                        let spec = fence::FenceSpec::from_sandbox(sb, meta.exec.net)
                            .with_read_only(self.fence_read_roots());
                        if let Err(e) = self.fence.release(&spec) {
                            self.log.warn(
                                "core::history_delete",
                                &format!("撤销围栏授权未完成：{}", e),
                            );
                        }
                    }
                }
                Err(e) => self.log.warn(
                    "core::history_delete",
                    &format!("取沙箱失败，未撤销授权：{}", e),
                ),
            }
        }
        self.sessions.remove(name);
        self.history.delete(name)
    }

    /// 事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        // 流式增量与工具调用实时事件都是短暂事件，不落盘；历史只记定稿后的行。
        let jsons: Vec<serde_json::Value> = events
            .iter()
            .filter(|e| !matches!(e, SessionEvent::Delta { .. } | SessionEvent::ToolCall(_)))
            .map(|e| e.to_json())
            .collect();
        if let Err(e) = self.history.append(sid, &jsons) {
            self.log.error(
                "core::history_append",
                &format!("会话 {} 落盘失败：{}", sid, e),
            );
            events.push(SessionEvent::Notice(format!(
                "[警告] 会话记录落盘失败：{}",
                e
            )));
        }
    }

    /// 核心推荐：按本次需求推荐 agent 名单（优先复用登记处的 agent，否则组装新的并给出模型）。
    /// 核心只建议、不代选；非法条目一律拒收（规则与代拟共用 agents::resolve_picks）。
    pub fn suggest_models(
        &self,
        task: &str,
        mode: WorkMode,
    ) -> Result<Vec<AgentSuggestion>, String> {
        if self.settings.models.is_empty() {
            return Err("登记处还没有任何模型，请先到「模型登记」添加".to_string());
        }
        let channel = self.settings.core_channel().ok_or_else(|| {
            "核心未设定默认模型（或它引用的供应商不存在），请先到「核心 AI 默认模型」设定"
                .to_string()
        })?;
        let roster = self.scan();
        // 形态描述文案在提示词册里（代码不硬编码给模型的说明）。
        let mode_text = match mode {
            WorkMode::Single => self.prompts.core.suggest_models.mode_single.clone(),
            WorkMode::Collab => self.prompts.core.suggest_models.mode_collab.clone(),
        };
        let user = self.prompts.render(
            &self.prompts.core.suggest_models.user,
            &[
                ("mode", mode_text.to_string()),
                (
                    "agents",
                    agents::listing(&self.prompts, &self.settings.agents),
                ),
                (
                    "modules",
                    module::listing(&roster, &self.prompts.core.tool_texts),
                ),
                (
                    "models",
                    agents::model_listing(&self.settings.models, &self.prompts.core.tool_texts),
                ),
                ("task", task.to_string()),
            ],
        );
        let (mut chat, _) = self.gateway.core_channel(Some(&channel));
        let raw = chat
            .complete(
                &[
                    Msg::system(self.prompts.core.suggest_models.system.clone()),
                    Msg::user(user),
                ],
                crate::core::ports::CompleteOpts::plain(false),
                &mut |_| true,
            )
            .raw;
        let parsed = envelope::extract_json_object(&raw)
            .and_then(|obj| serde_json::from_str::<SuggestReply>(&obj).ok())
            .ok_or_else(|| {
                format!(
                    "核心推荐失败（响应不是约定的 JSON）：{}",
                    raw.chars().take(200).collect::<String>()
                )
            })?;
        let (picks, rejected) = agents::resolve_picks(
            parsed.agents,
            &self.settings.agents,
            &roster,
            &self.settings.models,
        );
        for r in &rejected {
            self.log
                .warn("core::suggest_models", &format!("推荐条目拒收：{}", r));
        }
        let core_default = self.settings.core.clone();
        let drafts: Vec<AgentSuggestion> = picks
            .into_iter()
            .map(|(m, why)| AgentSuggestion {
                reuse: !m.transient,
                name: m.name,
                // 复用项没有自己的模型时，落到核心默认（前端仍可改）。
                model: m.model.or_else(|| core_default.clone()).unwrap_or_default(),
                modules: m.modules,
                why,
            })
            .collect();
        let out: Vec<AgentSuggestion> = match mode {
            // 单 agent：核心只给一条就原样采纳（模块数与 reuse 都保持它自己的）；
            // 给多条就把它们的模块并成一个临时 agent（并过的不是任何单个已存 agent，故 reuse=false）。
            WorkMode::Single => match drafts.len() {
                0 | 1 => drafts,
                _ => vec![merged_suggestion(drafts)],
            },
            // 协作 = N 个独立 agent，各带自己的模块与模型。
            WorkMode::Collab => drafts,
        };
        if out.is_empty() {
            return Err("核心推荐没有可用结果".to_string());
        }
        Ok(out)
    }

    // ---- 会话收发（前端永不接触会话本体） ----

    /// 形态**不钉在会话里**：每次生成前按登记处重新解析。
    /// 变了 → 走现有的重建路径刷新（系统提示随之换成另一套调用约定）并给用户一句通知；没变 → 什么都不做。
    /// 这样"用户改了登记处就重新查、没改就不管"，同时系统提示与实际协议始终一致（回放也按同一规则派生）。
    fn refresh_tool_mode(&mut self, sid: &str) -> Result<Option<String>, String> {
        let (meta, raw) = self.history.load(sid)?;
        let after = truncate_events(&raw);
        let Some(a) = meta.agents.first() else {
            return Ok(None);
        };
        let channel = a
            .model
            .as_deref()
            .and_then(|id| self.settings.resolve(id).ok())
            .or_else(|| self.settings.core_channel());
        let want = if channel.is_some() {
            self.settings.tool_mode_for(a.model.as_deref())
        } else {
            providers::ToolMode::Envelope
        };
        let cur = match self.sessions.get(sid) {
            // 还没装进内存的会话：交给 ensure_session 按当前形态建，这里不动
            Some(Session::Single(s)) => s.tool_mode(),
            Some(_) => return Ok(None),
            None => want,
        };
        if cur == want {
            return Ok(None);
        }
        let rebuilt = self.rebuild_session(&meta, &after)?;
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(Some(match want {
            providers::ToolMode::Native => {
                "工具调用形态已按登记处改为**原生工具调用**（本条起生效）".to_string()
            }
            providers::ToolMode::Envelope => {
                "工具调用形态已按登记处改为**手写信封**（本条起生效）".to_string()
            }
        }))
    }

    /// 生成前的**准备**（短命令：只做检查与取出会话，不跑模型）。
    /// 语义与原来的 single_say / continue_flow 一致：工具形态变了先给一句提示；
    /// 继续时末条必须是用户发言（否则只提醒，不替用户发言）。
    pub(crate) fn prepare_single(
        &mut self,
        sid: &str,
        text: Option<&str>,
        want_stream: bool,
    ) -> Result<Prepared, String> {
        let llm = self.llm_opts(want_stream);
        // 还没装进内存的会话先从落盘重建（"继续"可能先于"打开"到达；真没这个会话仍然报无此会话）。
        self.ensure_session(sid)?;
        if matches!(self.sessions.get(sid), Some(Session::Collab(_))) {
            return Ok(Prepared::NotSingle);
        }
        let mut prefix: Vec<SessionEvent> = Vec::new();
        if let Some(n) = self.refresh_tool_mode(sid)? {
            prefix.push(SessionEvent::Notice(n));
        }
        if text.is_none() {
            let last_is_user =
                matches!(self.sessions.get(sid), Some(Session::Single(s)) if s.last_is_user());
            if !last_is_user {
                prefix.push(SessionEvent::Notice(NEED_USER.to_string()));
                return Ok(Prepared::Immediate(prefix));
            }
        }
        let session = self.take_single(sid)?;
        Ok(Prepared::Run {
            session: Box::new(session),
            prefix,
            llm,
        })
    }

    /// 测试用同步入口：与工作线程那条路**同一段语义**（准备 → 生成 → 交回落盘）。
    /// 生产路径不再走它——那里的生成在工作线程上（见 `CoreHandle::single_generation`）。
    #[cfg(test)]
    pub fn single_say(
        &mut self,
        sid: &str,
        text: &str,
        live: &mut Live,
    ) -> Result<Vec<SessionEvent>, String> {
        match self.prepare_single(sid, Some(text), live.llm.stream)? {
            Prepared::Immediate(events) => Ok(events),
            Prepared::NotSingle => Err("该会话不是单 agent 模式".to_string()),
            Prepared::Run {
                session, prefix, ..
            } => {
                let mut session = *session;
                let mut events = prefix;
                events.extend(session.say(text, live));
                if live.cancelled() {
                    self.log.warn("core::single_say", "生成被用户中止");
                }
                self.put_single(sid, session, &events);
                Ok(events)
            }
        }
    }

    /// 协作推进一步：由前端按 pending 驱动；返回期间产生的全部事件。
    pub fn collab_continue(
        &mut self,
        sid: &str,
        step: CollabStep,
        text: &str,
    ) -> Result<Vec<SessionEvent>, String> {
        let mut out = Vec::new();
        let mut confirmed: Option<Vec<AgentMeta>> = None;
        {
            let s = self.sessions.get_mut(sid).ok_or("无此会话")?;
            let collab = match s {
                Session::Collab(c) => c,
                _ => return Err("该会话不是协作模式".to_string()),
            };
            match step {
                CollabStep::SetTask => collab.set_task(text, &mut |e| out.push(e)),
                CollabStep::ConfirmSlate => {
                    collab.confirm_slate(text.eq_ignore_ascii_case("yes"), &mut |e| out.push(e));
                    if !collab.roster().is_empty() {
                        confirmed = Some(collab.roster().to_vec());
                    }
                }
                CollabStep::Begin => collab.begin(text.contains("allow"), &mut |e| out.push(e)),
                CollabStep::Answer => collab.answer(text, &mut |e| out.push(e)),
            }
        }
        // 名单刚定下来：落档 meta（重启/回档后 rebuild_session 从这里拿名单与沙箱归属）并建沙箱目录。
        if let Some(roster) = confirmed {
            let names: Vec<String> = roster.iter().map(|a| a.name.clone()).collect();
            self.workspace.prepare(sid, &names)?;
            let (mut meta, _) = self.history.load(sid)?;
            meta.agents = roster.clone();
            meta.modules = roster.iter().flat_map(|a| a.modules.clone()).collect();
            self.history.create(&meta)?;
            let module_roster = self.scan();
            let sandboxes = self.sandboxes(&meta, &module_roster)?;
            if let Some(Session::Collab(c)) = self.sessions.get_mut(sid) {
                c.set_sandboxes(sandboxes);
            }
        }
        // 会话终结后移出中心（前端据 Ended 回收）。
        if let Some(Session::Collab(c)) = self.sessions.get(sid) {
            if c.is_done() {
                self.sessions.remove(sid);
            }
        }
        self.record_events(sid, &mut out);
        Ok(out)
    }

    /// 代拟拟好的名单（待用户确认；CLI 把它当表单逐行打印）。
    pub fn collab_slate(&mut self, sid: &str) -> Result<Vec<AgentMeta>, String> {
        self.ensure_session(sid)?;
        match self.sessions.get(sid) {
            Some(Session::Collab(c)) => Ok(c.slate()),
            Some(_) => Err("该会话不是协作模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }

    /// 回档：保留到转录行 id 为止（含该行），其后记录一并删除；流水只追加 rewind 记录。
    /// 返回重放后的完整事件流，供前端整体重建（不用前端自己推算截断）。
    /// 单 agent 的活动会话按历史精确回退；协作与历史会话按转录重建（状态全部派生）。
    pub fn rewind(&mut self, sid: &str, keep_id: u64) -> Result<Vec<serde_json::Value>, String> {
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let precise = matches!(self.sessions.get(sid), Some(Session::Single(_)));
        if !precise {
            self.ensure_session(sid)?;
        }
        let (meta, raw_before) = self.history.load(sid)?;
        let before = truncate_events(&raw_before);
        self.history
            .append(
                sid,
                &[serde_json::json!({ "type": "rewind", "keep": keep_id })],
            )
            .map_err(|e| format!("回档落盘失败：{}", e))?;
        let (_, raw_after) = self.history.load(sid)?;
        let after = truncate_events(&raw_after);
        if precise {
            if let Some(Session::Single(s)) = self.sessions.get_mut(sid) {
                s.rewind(keep_id);
            }
        } else {
            let rebuilt = self.rebuild_session(&meta, &after)?;
            self.sessions.insert(sid.to_string(), rebuilt);
        }
        let dropped = crate::core::collab_state::tool_runs(&before)
            .saturating_sub(crate::core::collab_state::tool_runs(&after));
        let mut out = after;
        if dropped > 0 {
            out.push(serde_json::json!({
                "type": "notice",
                "text": format!("[提示] 回档删掉了其后 {} 次工具执行——那些副作用不会回滚，继续可能会重新执行。", dropped),
            }));
        }
        Ok(out)
    }

    /// 协作中途改需求：回到需求行并追加一条新需求（旧需求留在流水里，派生以最后一条为准）。
    /// 返回重放后的完整事件流（前端整体重建，再吸收新产生的门事件）。
    pub fn update_task(&mut self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("需求不能为空".to_string());
        }
        self.ensure_session(sid)?;
        let (_, events) = self.history_open(sid)?;
        let keep = find_line_id(&events, "[用户:需求]").ok_or("该会话没有需求行")?;
        // 回档语义是「保留 id < keep」：需求行本身要留下（旧需求留在流水里），所以传 keep + 1。
        let mut out = self.rewind(sid, keep + 1)?;
        let mut fresh = Vec::new();
        {
            let s = self.sessions.get_mut(sid).ok_or("无此会话")?;
            match s {
                Session::Collab(c) => c.set_task(text, &mut |e| fresh.push(e)),
                _ => return Err("只有协作会话有「本次需求」".to_string()),
            }
        }
        self.record_events(sid, &mut fresh);
        out.extend(fresh.iter().map(|e| e.to_json()));
        Ok(out)
    }

    /// 撤回某 agent 的「同意」：转录追加一条撤回行并提醒（模型/用户都看得到）。
    pub fn withdraw_agree(&mut self, sid: &str, agent: &str) -> Result<Vec<SessionEvent>, String> {
        self.ensure_session(sid)?;
        let mut events = Vec::new();
        match self.sessions.get_mut(sid) {
            Some(Session::Collab(c)) => c.withdraw_agree(agent, &mut |e| events.push(e)),
            Some(_) => return Err("只有协作会话有「同意」可撤回".to_string()),
            None => return Err("无此会话".to_string()),
        }
        self.record_events(sid, &mut events);
        Ok(events)
    }

    /// 历史会话转为活动会话（跨重启续跑/回档的前提）：按转录重建，状态全部派生。
    fn ensure_session(&mut self, sid: &str) -> Result<(), String> {
        if self.sessions.contains_key(sid) {
            return Ok(());
        }
        // 会话对象在工作线程上（生成中）：**不能**从盘上再建一份（会变成两个实例）。
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, events) = self.history_open(sid)?;
        let rebuilt = self.rebuild_session(&meta, &events)?;
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(())
    }

    /// 按会话元信息 + 转录事件重建会话对象（通道是机制，按记录的选择重新装配）。
    fn rebuild_session(
        &self,
        meta: &SessionMeta,
        events: &[serde_json::Value],
    ) -> Result<Session, String> {
        let roster = self.source.scan();
        let sandboxes = self.sandboxes(meta, &roster)?;
        match meta.mode.as_str() {
            "collab" => Ok(Session::Collab(CollabSession::restore(
                Arc::clone(&self.gateway),
                Arc::clone(&self.source),
                self.settings.clone(),
                self.prompts.clone(),
                Arc::clone(&self.tools),
                Arc::clone(&self.io),
                Arc::clone(&self.repair),
                Arc::clone(&self.log),
                Arc::clone(&self.packages),
                meta,
                events,
                sandboxes,
            )?)),
            // 单 agent：按 meta.agents[0] 重建（名单是唯一真相；模块数不限）。
            "single" => {
                let a = meta
                    .agents
                    .first()
                    .ok_or_else(|| format!("会话 {} 缺少 agent 名单", meta.name))?;
                let modules: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| {
                        roster
                            .modules
                            .iter()
                            .find(|m| &m.manifest.id == id)
                            .cloned()
                    })
                    .collect();
                if modules.len() != a.modules.len() {
                    return Err(format!("agent {} 的模块已不在清单", a.name));
                }
                let sb = sandboxes.for_agent(&a.name).cloned().ok_or_else(|| {
                    format!("会话 {} 缺少 agent {} 的沙箱信息", meta.name, a.name)
                })?;
                let channel = a
                    .model
                    .as_deref()
                    .and_then(|id| self.settings.resolve(id).ok())
                    .or_else(|| self.settings.core_channel());
                // 重建时同样按登记处派生形态：系统提示与实际协议必须一致（回放才与实时一致）
                let mode = if channel.is_some() {
                    self.settings.tool_mode_for(a.model.as_deref())
                } else {
                    providers::ToolMode::Envelope
                };
                let guide = systool::guide(&self.prompts, &sb);
                let system = module::agent_system(&self.prompts, &a.name, &modules, &guide, mode);
                let (chat, note) = self.gateway.member_channel(channel.as_ref(), &a.name);
                // 先把转录行按顺序摊平：分组判断要看「下一行是不是 tool 行」。
                let mut rows: Vec<&serde_json::Value> = Vec::new();
                for ev in events {
                    if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
                        continue;
                    }
                    if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
                        rows.extend(lines.iter());
                    }
                }
                let mut history = vec![Msg::system(system)];
                let mut marks: Vec<usize> = Vec::new();
                let mut line_reply: Vec<u64> = Vec::new();
                let texts = &self.prompts.core.tool_texts;
                let reply_of =
                    |v: &serde_json::Value| v.get("reply").and_then(|x| x.as_u64()).unwrap_or(0);
                let mut i = 0usize;
                while i < rows.len() {
                    let l = rows[i];
                    let line = l.get("line").and_then(|x| x.as_str()).unwrap_or("");
                    if let Some(t) = line.strip_prefix("[用户] ") {
                        history.push(Msg::user(t.to_string()));
                        // 用户行不属于任何回复：给它自己的行号，回档时才不会与相邻行误并成一组。
                        line_reply.push(l.get("id").and_then(|x| x.as_u64()).unwrap_or(0));
                        marks.push(history.len());
                        i += 1;
                    } else if l.get("tool").is_some() {
                        // 【回复分组 · 改动前务必读完】**同一次回复的 tool 行连续同号**（reply 由引擎给、
                        // 落行时写入）；整组一起翻译成消息，靠的正是这个号——不靠"相邻行猜分组"。
                        let reply = reply_of(l.get("tool").expect("已判存在"));
                        let mut group: Vec<&serde_json::Value> = Vec::new();
                        while i < rows.len()
                            && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null))
                                == reply
                        {
                            group.push(rows[i]);
                            i += 1;
                        }
                        // 这一回复的助手消息正文（空正文的回复不带 raw；组内取一份即可）。
                        let raw = group
                            .iter()
                            .find_map(|t| {
                                t.get("tool")
                                    .and_then(|x| x.get("raw"))
                                    .and_then(|x| x.as_str())
                                    .filter(|s| !s.is_empty())
                            })
                            .unwrap_or_default();
                        let views: Vec<crate::core::events::ToolCallView> = group
                            .iter()
                            .filter_map(|t| t.get("tool").cloned())
                            .filter_map(|t| serde_json::from_value(t).ok())
                            .collect();
                        for m in crate::core::engine::reply_msgs(mode, raw, &views, texts) {
                            history.push(m);
                        }
                        for _ in 0..group.len() {
                            line_reply.push(reply);
                            marks.push(history.len());
                        }
                    } else {
                        // 文本行：它紧跟 tool 行时属于同一次回复（历史由那组 tool 行统一推进，这里不推）；
                        // 否则这一行自己就是一条回复，推 assistant(该行文本)。
                        let next_is_tool = rows
                            .get(i + 1)
                            .map(|n| n.get("tool").is_some())
                            .unwrap_or(false);
                        if !next_is_tool {
                            let text = line
                                .split_once("] ")
                                .map(|(_, t)| t)
                                .unwrap_or(line)
                                .to_string();
                            history.push(Msg::assistant(text));
                        }
                        line_reply.push(reply_of(l));
                        marks.push(history.len());
                        i += 1;
                    }
                }
                let unavailable = self.unavailable_modules(&meta.exec, &modules);
                let mut tools = self.tools_env(&modules, &sb, unavailable, meta.exec.net, mode);
                // 回复 id 跨重启单调：从转录里的最大值续号，否则新回复会与旧回复并成一组。
                tools.reply_seq = crate::core::engine::max_reply(events);
                let roots = crate::core::refs::RefRoots {
                    work: sb.shared.clone(),
                    private: Some(sb.private.clone()),
                };
                Ok(Session::Single(session::AgentSession::restore(
                    &a.name,
                    history,
                    marks,
                    line_reply,
                    chat,
                    note,
                    Some(tools),
                    self.prompts.core.refs.clone(),
                    roots,
                    self.prompts.core.tool_texts.clone(),
                )))
            }
            other => Err(format!("未知会话形态：{}（只认 single / collab）", other)),
        }
    }

    /// 继续：由用户点击授权。单 agent 会话需要轮到用户（末条是 AI 就只提醒、不发请求）；
    /// 协作不需要用户发言，继续 = 从断点推进流水线。
    /// **测试用同步入口**：生产路径的两条（单 agent / 协作）都在工作线程上跑（见 CoreHandle）。
    #[cfg(test)]
    pub fn continue_flow(
        &mut self,
        sid: &str,
        live: &mut Live,
    ) -> Result<Vec<SessionEvent>, String> {
        self.ensure_session(sid)?;
        let mut events = {
            let s = self.sessions.get_mut(sid).ok_or("无此活动会话")?;
            match s {
                Session::Single(s) => {
                    if s.last_is_user() {
                        s.continue_reply(live)
                    } else {
                        vec![SessionEvent::Notice(NEED_USER.to_string())]
                    }
                }
                // 协作不需要用户发言：从断点推进流水线。
                Session::Collab(c) => {
                    let mut out = Vec::new();
                    c.resume(&mut |e| out.push(e));
                    out
                }
            }
        };
        self.record_events(sid, &mut events);
        Ok(events)
    }

    /// 协作会话当前介入请求（None = 无挂起或已终结）。
    pub fn collab_pending(&self, sid: &str) -> Result<Option<Pending>, String> {
        match self.sessions.get(sid) {
            Some(Session::Collab(c)) => Ok(c.pending.clone()),
            Some(_) => Err("该会话不是协作模式".to_string()),
            None => Err("无此会话".to_string()),
        }
    }
}

// 测试访问器：验证职责提示词已种入历史首条（回归：会话曾丢失 system 提示词）。
#[cfg(test)]
impl Core {
    pub fn single_history(&self, sid: &str) -> Option<Vec<Msg>> {
        match self.sessions.get(sid) {
            Some(Session::Single(s)) => Some(s.history().to_vec()),
            _ => None,
        }
    }
}

/// 继续被拦下时的提醒（单 agent 会话：该轮到用户发言）。
const NEED_USER: &str = "最后一条是 AI 发言：部分供应商不允许连续 AI 发言；请先发言再继续。";

/// 应用流水里的 rewind 记录：会话内容 = 回放时只保留「id < keep」的转录行（回档 = 删该行及其后）。
fn truncate_events(events: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut content: Vec<serde_json::Value> = Vec::new();
    for ev in events {
        match ev.get("type").and_then(|t| t.as_str()) {
            Some("rewind") => {
                // 缺 keep 字段 = 不截断（安全默认：宁可多留，不可清空一切）。
                if let Some(keep) = ev.get("keep").and_then(|k| k.as_u64()) {
                    content = cut_before_line(&content, keep);
                }
            }
            _ => content.push(ev.clone()),
        }
    }
    content
}

/// 只保留「转录行 id < keep」的行（回档 = 删除该行及其后；keep = 0 → 转录清空）。
/// 一旦某条事件里的行被截掉，其后的事件一并丢弃（事件流是时序的）。
fn cut_before_line(events: &[serde_json::Value], keep: u64) -> Vec<serde_json::Value> {
    let keep = align_keep(events, keep);
    let mut out = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) == Some("transcript") {
            if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
                let kept: Vec<serde_json::Value> = lines
                    .iter()
                    .take_while(|l| {
                        l.get("id")
                            .and_then(|i| i.as_u64())
                            .map(|i| i < keep)
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                let done = kept.len() < lines.len();
                if !kept.is_empty() {
                    let mut e2 = ev.clone();
                    e2["lines"] = serde_json::Value::Array(kept);
                    out.push(e2);
                }
                if done {
                    return out;
                }
            }
        } else {
            out.push(ev.clone());
        }
    }
    out
}

/// 把"保留 id < keep"对齐到**回复边界**（见 session::keep_whole_replies）：
/// keep 落在某次回复内部时退到该回复第一行之前，返回新的 keep（没有这样的行 = u64::MAX，即不截）。
fn align_keep(events: &[serde_json::Value], keep: u64) -> u64 {
    let rows: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flatten()
        .collect();
    let idx = rows
        .iter()
        .position(|l| {
            l.get("id")
                .and_then(|i| i.as_u64())
                .map(|i| i >= keep)
                .unwrap_or(false)
        })
        .unwrap_or(rows.len());
    let replies: Vec<u64> = rows.iter().map(|l| line_reply_of(l)).collect();
    let aligned = crate::core::session::keep_whole_replies(&replies, idx);
    rows.get(aligned)
        .and_then(|l| l.get("id").and_then(|i| i.as_u64()))
        .unwrap_or(u64::MAX)
}

/// 一行属于哪次回复：工具行的号在调用视图里，其余行在 LineView 上。
/// 号 = 0 视为"没写"（回复号从 1 起），此时用**该行自己的 id**——绝不把相邻行误并成一组。
fn line_reply_of(l: &serde_json::Value) -> u64 {
    let own = l.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
    let stored = match l.get("tool") {
        Some(t) => t.get("reply").and_then(|x| x.as_u64()),
        None => l.get("reply").and_then(|x| x.as_u64()),
    };
    stored.filter(|r| *r != 0).unwrap_or(own)
}

/// 找最后一条以 prefix 开头的转录行的 id。
fn find_line_id(events: &[serde_json::Value], prefix: &str) -> Option<u64> {
    let mut found = None;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            if l.get("line")
                .and_then(|x| x.as_str())
                .map(|s| s.starts_with(prefix))
                .unwrap_or(false)
            {
                found = l.get("id").and_then(|i| i.as_u64());
            }
        }
    }
    found
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
    }
}

/// 把多条推荐项并成一个临时 agent（模块按出现顺序去重、模型取首项、理由合并）。
/// 单 agent 形态拿到多条推荐时用它收口：并过的不是任何单个已存 agent，故 reuse = false。
fn merged_suggestion(drafts: Vec<AgentSuggestion>) -> AgentSuggestion {
    let mut modules: Vec<String> = Vec::new();
    let mut model = String::new();
    let mut why: Vec<String> = Vec::new();
    for a in drafts {
        if model.is_empty() {
            model = a.model;
        }
        for id in a.modules {
            if !modules.contains(&id) {
                modules.push(id);
            }
        }
        if !a.why.trim().is_empty() {
            why.push(a.why);
        }
    }
    AgentSuggestion {
        name: "组合".to_string(),
        modules,
        model,
        why: why.join("；"),
        reuse: false,
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
