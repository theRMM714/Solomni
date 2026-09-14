//! 核心层：定义抽象（ports）、编排业务（会话/引擎）、会话中心。
//! 分层纪律：本层不出现文件读、ureq、stdin/stdout——机制全部在 adapters，
//! 装配（new 适配器）只发生在 main 组合根。前端只见 Core 门面、会话句柄与 SessionEvent 流。

pub mod agents;
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
pub mod ports;
pub mod prompt;
pub mod providers;
pub mod refs;
pub mod session;
pub mod systool;
pub mod workspace;

pub use events::{Live, Pending, SessionEvent};
pub use ports::{
    ChatGateway, HistoryStore, ModelCatalog, ModuleSource, PackageSource, PromptSource, SettingsStore, SysIo, ToolRunner,
    Workspace,
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
    log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    settings: Settings,
    prompts: Prompts,
    sessions: HashMap<SessionId, Session>,
}

impl Core {
    /// 组合根专用：main 负责创建适配器并注入；core 不自建任何具体实现。
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
        prompt_source: Box<dyn PromptSource>,
        log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    ) -> Result<Core, String> {
        let log_for_core = Arc::clone(&log);
        let outcome = (|| -> Result<Core, String> {
            let settings = store.load()?;
            let prompts = prompt_source.load()?;
            Ok(Core { store, history, workspace, source, packages, fence, gateway, catalog, tools, io, log: log_for_core, settings, prompts, sessions: HashMap::new() })
        })();
        if let Err(e) = &outcome {
            log.error("core::new", &format!("装配失败：{}", e)); // 仅错误时借用，不与闭包 move 冲突
        }
        outcome
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
        let spec = exec::ExecSpec { tier, ..exec::ExecSpec::default() };
        let diagnoses = if tier == exec::Tier::Vm {
            exec::vm_diagnoses(&roster.modules, &lib, &spec)
        } else {
            Vec::new()
        };
        RuntimeReport {
            tier: tier.as_str().to_string(),
            declared: exec::declared(&roster.modules),
            available: lib.capability_versions(),
            missing: exec::absent(&roster.modules, &lib),
            diagnoses,
            rejected: roster.rejected.clone(),
            rejected_packages: lib.rejected.clone(),
        }
    }

    /// 本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
    fn unavailable_modules(&self, spec: &exec::ExecSpec, modules: &[Module]) -> BTreeMap<String, Vec<String>> {
        exec::unavailable(spec, modules, &self.packages.scan())
    }

    /// 配置视图：把「能改什么、现在是什么、缺什么」如实给出（每次读取都重扫模块清单与包库）。
    pub fn session_config(&self, sid: &str) -> Result<SessionConfig, String> {
        let (meta, events) = self.history_open(sid)?;
        let tier = meta.exec.tier;
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
        })
    }

    /// 编辑提交：校验 → 写回 meta.yaml（名单与选型的唯一真相）→ 追加一条旁路配置记录 → 丢掉内存会话。
    /// 生效点：下一次访问按新配置从转录重建会话对象（所以改完不必重开会话）。
    /// 冻结：流水里有内容（会话已经开过）时，agent 名单与形态不可改——换人请新建会话。
    pub fn edit_session(&mut self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        let (meta, events) = self.history_open(sid)?;
        match meta.mode.as_str() {
            "single" | "collab" => {}
            other => return Err(format!("未知会话形态：{}（只认 single / collab）", other)),
        }
        if session_started(&events) {
            let old: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
            let new: Vec<String> = edit.agents.iter().map(|a| a.name.clone()).collect();
            if old != new {
                return Err("这轮会话已经开过：agent 名单与形态冻结（要换人请新建会话）".to_string());
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
                    return Err(format!("模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent", id));
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
                model: if a.model.is_empty() { None } else { Some(a.model.clone()) },
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
        let spec = exec::ExecSpec { tier, base: edit.base.clone(), pins: edit.pins.clone(), net: edit.net };
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| seen.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        let plan = exec::plan(&spec, &session_modules, &self.packages.scan()).map_err(|diags| exec::diagnose_text(&diags))?;
        self.log.info("core::edit_session", &format!("sid={}；{}", sid, exec::plan_summary(&plan)));

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
            self.log.warn("core::record_config", &format!("配置记录落盘失败：{}", e));
        }
    }

    // ---- 登记处：供应商（密钥只在此层进出；前端只见 id 与端点） ----

    /// 结构化供应商视图（不含密钥；Web 用）。
    pub fn provider_views(&self) -> Vec<providers::ProviderView> {
        self.settings.provider_views()
    }

    /// CLI 展示行：供应商（不含密钥）。
    pub fn provider_lines(&self) -> Vec<String> {
        self.settings.provider_lines()
    }

    /// 新建/更新供应商。更新时 api_key 留空 = 保留原密钥（界面从不回显密钥）。
    pub fn provider_upsert(&mut self, id: &str, base_url: &str, api_key: &str) -> Result<(), String> {
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
            providers::Provider { kind: "llm".to_string(), base_url: base_url.to_string(), api_key: key },
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
            return Err(format!("供应商 {} 仍被模型引用：{}；请先删除这些模型", id, referenced.join("、")));
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

    /// CLI 展示行：模型。
    pub fn model_lines(&self) -> Vec<String> {
        self.settings.model_lines()
    }

    /// 核心 AI 默认模型 id。
    pub fn core_model(&self) -> Option<String> {
        self.settings.core.clone()
    }

    pub fn model_upsert(&mut self, id: &str, name: &str, api_model: &str, provider: &str, note: &str) -> Result<(), String> {
        if id.is_empty() || name.is_empty() || api_model.is_empty() || provider.is_empty() {
            return Err("id / name / api_model / provider 均不能为空".to_string());
        }
        if !self.settings.providers.contains_key(provider) {
            return Err(format!("无此供应商：{}", provider));
        }
        self.settings.models.insert(
            id.to_string(),
            providers::ModelEntry {
                name: name.to_string(),
                api_model: api_model.to_string(),
                provider: provider.to_string(),
                note: note.to_string(),
            },
        );
        self.save_settings("core::model_upsert")
    }

    /// 删除模型；是核心默认模型时拒绝（先改默认再删）。
    pub fn model_remove(&mut self, id: &str) -> Result<bool, String> {
        if self.settings.core.as_deref() == Some(id) {
            return Err(format!("{} 是核心默认模型；请先把核心默认模型改成别的再删", id));
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
    pub fn agent_upsert(&mut self, name: &str, module_ids: &[String], model: &str, note: &str) -> Result<(), String> {
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
                model: if model.is_empty() { None } else { Some(model.to_string()) },
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

    pub fn set_app_settings(&mut self, app: AppSettings) -> Result<(), String> {
        self.settings.app = app;
        self.save_settings("core::set_app_settings")
    }

    /// 用登记处已存的供应商去拉取其可用模型名（发现机制在适配层）。
    pub fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let provider = self.settings.providers.get(provider_id).ok_or_else(|| format!("无此供应商：{}", provider_id))?;
        let outcome = self.catalog.list_models(provider);
        match &outcome {
            Ok(models) => self.log.info("core::discover_models", &format!("供应商 {} 拉取模型 {} 个", provider_id, models.len())),
            Err(e) => self.log.error("core::discover_models", &format!("供应商 {} 拉取模型失败：{}", provider_id, e)),
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
        self.sessions
            .iter()
            .map(|(sid, s)| {
                let done = match s {
                    Session::Collab(c) => c.is_done(),
                    Session::Single(_) => false,
                };
                let mode = history.iter().find(|h| &h.name == sid).map(|h| h.mode.clone()).unwrap_or_default();
                SessionView { sid: sid.clone(), mode, done }
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
                    return Err(format!("模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent", id));
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
            metas.push(AgentMeta { name, transient: a.transient, modules: a.modules.clone(), model: a.model.clone() });
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
            exec: exec::ExecSpec { tier: self.settings.app.tier, ..exec::ExecSpec::default() },
        };
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
            self.log.warn("core::create_work", &format!("模块 {} 声明的运行包不在包库：{}", id, caps.join("、")));
        }
        // 执行选型的完整性检查（「开始」即冻结）：虚拟机档的选型不成立（多版本未定版 / 定版不存在 /
        // 路径冲突）如实拒绝；只是缺包的照常开始——那是该模块的工具不可用（降级而非崩溃）。装配阶段按同一份计划取包。
        let plan = exec::plan(&meta.exec, &session_modules, &self.packages.scan())
            .map_err(|diags| exec::diagnose_text(&diags))?;
        self.log.info("core::create_work", &exec::plan_summary(&plan));

        let (session, mut events) = match spec.mode {
            // 单 agent（模块数不限）。
            WorkMode::Single => {
                let a = metas.first().ok_or("至少要有一个 agent")?;
                let sb = sandboxes.for_agent(&a.name).cloned().ok_or_else(|| format!("缺少 agent {} 的沙箱", a.name))?;
                let chosen: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| roster.modules.iter().find(|m| &m.manifest.id == id).cloned())
                    .collect();
                let channel = self.channel_of(a.model.as_deref());
                let unavailable = self.unavailable_modules(&meta.exec, &chosen);
                let (s, opened) = self.build_single(a, &chosen, channel, &sb, unavailable, meta.exec.net);
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
        Ok(WorkOpened { sid: name, agents: agent_names, events })
    }

    /// 界面投喂：把文件写进本次工作的 work/。
    /// 返回 Ok(false) = 同名文件已存在且未选择覆盖（交前端让用户决定：覆盖/改名/取消）。
    pub fn work_upload(&mut self, sid: &str, name: &str, bytes: &[u8], overwrite: bool) -> Result<bool, String> {
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
        let (meta, _) = self.history.load(sid).map_err(|_| format!("无此会话：{}", sid))?;
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
            roots: FilesRootsView { work: workspace::slash(&sandboxes.shared), agents: agent_roots },
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
    fn sandboxes(&self, meta: &SessionMeta, roster: &module::Roster) -> Result<workspace::Sandboxes, String> {
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
            });
        }
        Ok(workspace::Sandboxes { shared: roots.shared, list })
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
    ) -> engine::MemberTools {
        engine::MemberTools {
            modules: engine::tool_table(modules),
            runner: Arc::clone(&self.tools),
            sandbox: sb.clone(),
            io: Arc::clone(&self.io),
            unavailable,
            // 围栏：可达范围 + 断网 + 环境白名单的落点，全部由该 agent 的沙箱派生（机制在 adapters）。
            fence: crate::core::fence::FenceSpec::from_sandbox(sb, net),
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
        self.log.info(
            "core::build_single",
            &format!(
                "单 agent 工作：agent {}，模块 {}，模型 {}",
                a.name,
                a.modules.join("+"),
                channel.as_ref().map(|c| c.model.as_str()).unwrap_or("无（演示）")
            ),
        );
        let system = module::agent_system(&self.prompts, &a.name, modules, &systool::guide(&self.prompts, sb));
        let tools = self.tools_env(modules, sb, unavailable, net);
        let roots = crate::core::refs::RefRoots { work: sb.shared.clone(), private: Some(sb.private.clone()) };
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
                self.log.error("core::history_list", &format!("会话列表失败：{}", e));
                Vec::new()
            }
        }
    }

    /// 打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
    /// 回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
    pub fn history_open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let (meta, events) = self.history.load(name)?;
        Ok((meta, truncate_events(&events)))
    }

    /// 删除会话（= 删目录）。内存中的同名会话一并移除，避免内存与磁盘不一致。
    /// 删之前先请适配层撤销该会话各 agent 的围栏授权：痕迹与会话同生共死，不随会话数量堆积。
    pub fn history_delete(&mut self, name: &str) -> Result<bool, String> {
        if let Ok((meta, _)) = self.history.load(name) {
            let roster = self.source.scan();
            match self.sandboxes(&meta, &roster) {
                Ok(sandboxes) => {
                    for sb in &sandboxes.list {
                        let spec = fence::FenceSpec::from_sandbox(sb, meta.exec.net);
                        if let Err(e) = self.fence.release(&spec) {
                            self.log.warn("core::history_delete", &format!("撤销围栏授权未完成：{}", e));
                        }
                    }
                }
                Err(e) => self.log.warn("core::history_delete", &format!("取沙箱失败，未撤销授权：{}", e)),
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
            self.log.error("core::history_append", &format!("会话 {} 落盘失败：{}", sid, e));
            events.push(SessionEvent::Notice(format!("[警告] 会话记录落盘失败：{}", e)));
        }
    }

    /// 核心推荐：按本次需求推荐 agent 名单（优先复用登记处的 agent，否则组装新的并给出模型）。
    /// 核心只建议、不代选；非法条目一律拒收（规则与代拟共用 agents::resolve_picks）。
    pub fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String> {
        if self.settings.models.is_empty() {
            return Err("登记处还没有任何模型，请先到「模型登记」添加".to_string());
        }
        let channel = self
            .settings
            .core_channel()
            .ok_or_else(|| "核心未设定默认模型（或它引用的供应商不存在），请先到「核心 AI 默认模型」设定".to_string())?;
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
                ("agents", agents::listing(&self.prompts, &self.settings.agents)),
                ("modules", module::listing(&roster, &self.prompts.core.tool_texts)),
                ("models", agents::model_listing(&self.settings.models, &self.prompts.core.tool_texts)),
                ("task", task.to_string()),
            ],
        );
        let (mut chat, _) = self.gateway.core_channel(Some(&channel));
        let raw = chat.complete(
            &[Msg::system(self.prompts.core.suggest_models.system.clone()), Msg::user(user)],
            false,
            &mut |_| true,
        );
        let parsed = envelope::extract_json_object(&raw)
            .and_then(|obj| serde_json::from_str::<SuggestReply>(&obj).ok())
            .ok_or_else(|| format!("核心推荐失败（响应不是约定的 JSON）：{}", raw.chars().take(200).collect::<String>()))?;
        let (picks, rejected) = agents::resolve_picks(parsed.agents, &self.settings.agents, &roster, &self.settings.models);
        for r in &rejected {
            self.log.warn("core::suggest_models", &format!("推荐条目拒收：{}", r));
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

    /// 单 agent 会话发言。
    pub fn single_say(&mut self, sid: &str, text: &str, live: &mut Live) -> Result<Vec<SessionEvent>, String> {
        let mut events = match self.sessions.get_mut(sid) {
            Some(Session::Single(s)) => s.say(text, live),
            Some(_) => return Err("该会话不是单 agent 模式".to_string()),
            None => return Err("无此会话".to_string()),
        };
        if live.cancelled() {
            self.log.warn("core::single_say", "生成被用户中止");
        }
        self.record_events(sid, &mut events);
        Ok(events)
    }

    /// 协作推进一步：由前端按 pending 驱动；返回期间产生的全部事件。
    pub fn collab_continue(&mut self, sid: &str, step: CollabStep, text: &str) -> Result<Vec<SessionEvent>, String> {
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
        let precise = matches!(self.sessions.get(sid), Some(Session::Single(_)));
        if !precise {
            self.ensure_session(sid)?;
        }
        let (meta, raw_before) = self.history.load(sid)?;
        let before = truncate_events(&raw_before);
        self.history
            .append(sid, &[serde_json::json!({ "type": "rewind", "keep": keep_id })])
            .map_err(|e| format!("回档落盘失败：{}", e))?;
        let (_, raw_after) = self.history.load(sid)?;
        let after = truncate_events(&raw_after);
        if precise {
            match self.sessions.get_mut(sid) {
                Some(Session::Single(s)) => s.rewind(keep_id),
                _ => {}
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
        let (meta, events) = self.history_open(sid)?;
        let rebuilt = self.rebuild_session(&meta, &events)?;
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(())
    }

    /// 按会话元信息 + 转录事件重建会话对象（通道是机制，按记录的选择重新装配）。
    fn rebuild_session(&self, meta: &SessionMeta, events: &[serde_json::Value]) -> Result<Session, String> {
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
                Arc::clone(&self.packages),
                meta,
                events,
                sandboxes,
            )?)),
            // 单 agent：按 meta.agents[0] 重建（名单是唯一真相；模块数不限）。
            "single" => {
                let a = meta.agents.first().ok_or_else(|| format!("会话 {} 缺少 agent 名单", meta.name))?;
                let modules: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| roster.modules.iter().find(|m| &m.manifest.id == id).cloned())
                    .collect();
                if modules.len() != a.modules.len() {
                    return Err(format!("agent {} 的模块已不在清单", a.name));
                }
                let sb = sandboxes
                    .for_agent(&a.name)
                    .cloned()
                    .ok_or_else(|| format!("会话 {} 缺少 agent {} 的沙箱信息", meta.name, a.name))?;
                let guide = systool::guide(&self.prompts, &sb);
                let system = module::agent_system(&self.prompts, &a.name, &modules, &guide);
                let channel = a
                    .model
                    .as_deref()
                    .and_then(|id| self.settings.resolve(id).ok())
                    .or_else(|| self.settings.core_channel());
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
                for (i, l) in rows.iter().enumerate() {
                    let line = l.get("line").and_then(|x| x.as_str()).unwrap_or("");
                    if let Some(t) = line.strip_prefix("[用户] ") {
                        history.push(Msg::user(t.to_string()));
                    } else if let Some(tool) = l.get("tool") {
                        // tool 行：重建该轮模型原始输出 + 回注的工具结果（两个都要，否则上下文缺一块）。
                        let field = |k: &str| tool.get(k).and_then(|x| x.as_str()).unwrap_or("");
                        history.push(Msg::assistant(field("raw").to_string()));
                        let module = field("module");
                        let name = field("name");
                        let label = if module.is_empty() { name.to_string() } else { format!("{}.{}", module, name) };
                        let texts = &self.prompts.core.tool_texts;
                        history.push(Msg::user(texts.render(
                            &texts.tool_result_wrapper,
                            &[("label", label), ("output", field("output").to_string())],
                        )));
                    } else {
                        // 【分组规则 · 改动前务必读完】工具轮可能产出「文本行 + tool 行」两行，
                        // 但这一轮在历史里只有一条 assistant(raw)（raw 含正文+信封）——由 tool 行统一推进。
                        // 所以：**若某文本行紧跟一条 tool 行（同一轮），这里不推 assistant**；
                        // 只有「后面不是 tool 行」的文本行才推 assistant(该行文本)。
                        // 否则重建出来的上下文会凭空多一条 assistant，与实时历史不一致。
                        let next_is_tool = rows.get(i + 1).map(|n| n.get("tool").is_some()).unwrap_or(false);
                        if !next_is_tool {
                            let text = line.split_once("] ").map(|(_, t)| t).unwrap_or(line).to_string();
                            history.push(Msg::assistant(text));
                        }
                    }
                    marks.push(history.len());
                }
                let unavailable = self.unavailable_modules(&meta.exec, &modules);
                let tools = self.tools_env(&modules, &sb, unavailable, meta.exec.net);
                let roots = crate::core::refs::RefRoots { work: sb.shared.clone(), private: Some(sb.private.clone()) };
                Ok(Session::Single(session::AgentSession::restore(
                    &a.name,
                    history,
                    marks,
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
    pub fn continue_flow(&mut self, sid: &str, live: &mut Live) -> Result<Vec<SessionEvent>, String> {
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
    let mut out = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) == Some("transcript") {
            if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
                let kept: Vec<serde_json::Value> = lines
                    .iter()
                    .take_while(|l| l.get("id").and_then(|i| i.as_u64()).map(|i| i < keep).unwrap_or(false))
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

/// 找最后一条以 prefix 开头的转录行的 id。
fn find_line_id(events: &[serde_json::Value], prefix: &str) -> Option<u64> {
    let mut found = None;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else { continue };
        for l in lines {
            if l.get("line").and_then(|x| x.as_str()).map(|s| s.starts_with(prefix)).unwrap_or(false) {
                found = l.get("id").and_then(|i| i.as_u64());
            }
        }
    }
    found
}

/// 会话是否「已经开过」（流水里有内容）：agent 名单与形态据此冻结。配置记录与回档记录不算内容。
fn session_started(events: &[serde_json::Value]) -> bool {
    events.iter().any(|ev| match ev.get("type").and_then(|t| t.as_str()) {
        Some("config") | Some("rewind") => false,
        Some("transcript") => ev.get("lines").and_then(|l| l.as_array()).map(|l| !l.is_empty()).unwrap_or(false),
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
    AgentSuggestion { name: "组合".to_string(), modules, model, why: why.join("；"), reuse: false }
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
