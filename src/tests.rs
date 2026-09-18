//! 核心测试：全内存装配（InMemoryStore + VecSource + ScriptGateway），不碰文件系统。
//! 测试里的组合根 = 内存适配器；core 的可测性正是端口化的直接收益。
//! Web 阶段适配：适配器以 Arc 注入；会话经中心 id 收发；SharedScript 对齐真实通道时序。

use crate::adapters::fake_chat::FakeChat;
use crate::core::engine::{Discussion, Member, MemberTools, ModuleTools, TurnOut, MAX_ROUNDS, MAX_TOOL_CALLS};
use crate::core::module::{Module, ModuleManifest};
use crate::core::history::{AgentMeta, HistoryView, SessionMeta};
use crate::core::exec::{self, Diagnosis, ExecSpec, Tier};
use crate::core::packages::{Library, PackageManifest};
use crate::core::ports::{
    BoxedChat, Chat, ChatGateway, Chunk, FileRead, HistoryStore, ModelCatalog, ModuleSource, Msg, PackageSource,
    PromptSource, SettingsStore, SysIo, ToolOutcome, ToolRunner, Workspace,
};
use crate::core::prompt::{render, Prompts};
use crate::core::providers::{Channel, ModelEntry, Provider, Settings};
use crate::core::{
    AgentInstance, CollabStep, ConfigAgent, Core, Live, Pending, SessionEdit, SessionEvent, WorkMode, WorkSpec,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------- 内存适配器（测试组合根） ----------

/// 内存登记处：预置「供应商 p + 模型 m + 核心默认 m」，让会话都能拿到真实通道。
pub(crate) struct InMemorySettings {
    s: Mutex<Settings>,
    fail: Option<String>,
}

impl InMemorySettings {
    pub(crate) fn new() -> InMemorySettings {
        let mut s = Settings::default();
        s.providers.insert(
            "p".to_string(),
            Provider { base_url: "http://test".to_string(), api_key: "k".to_string() },
        );
        s.models.insert(
            "m".to_string(),
            ModelEntry { name: "M".to_string(), api_model: "m".to_string(), provider: "p".to_string(), note: String::new() },
        );
        s.core = Some("m".to_string());
        InMemorySettings { s: Mutex::new(s), fail: None }
    }

    /// 注入失败：load / save 一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemorySettings {
        self.fail = Some(msg.to_string());
        self
    }
    /// 指定默认执行档位的登记处（断言虚拟机档下的诊断与工具回执）。
    fn with_tier(tier: Tier) -> InMemorySettings {
        let s = InMemorySettings::new();
        s.s.lock().expect("锁").app.tier = tier;
        s
    }
}

impl SettingsStore for InMemorySettings {
    fn load(&self) -> Result<Settings, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self.s.lock().expect("锁").clone())
    }
    fn save(&self, s: &Settings) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        *self.s.lock().expect("锁") = s.clone();
        Ok(())
    }
}

/// 内存工作区：准备/写入都不碰盘，只记录（测试用）。
#[derive(Default)]
pub(crate) struct InMemoryWorkspace {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    fail: Option<String>,
}

impl InMemoryWorkspace {
    pub(crate) fn new() -> InMemoryWorkspace {
        InMemoryWorkspace { files: Mutex::new(BTreeMap::new()), fail: None }
    }

    /// 注入失败：prepare / roots / write_work / list 一律返回该原因（work_has 是布尔查询，不受影响）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryWorkspace {
        self.fail = Some(msg.to_string());
        self
    }
    /// 直接放一个文件（模拟落盘），键与真实布局同构：<session>/work/<名字> 或 <session>/<agent>/<相对路径>。
    pub(crate) fn seed(&self, session: &str, area: &str, rel: &str) {
        self.files.lock().expect("锁").insert(format!("{}/{}/{}", session, area, rel), Vec::new());
    }
}

impl Workspace for InMemoryWorkspace {
    fn prepare(&self, _session: &str, _agents: &[String]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(())
    }
    fn roots(&self, session: &str, agents: &[String]) -> Result<crate::core::workspace::WorkRoots, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 与 FsWorkspace 同构的**绝对**路径（以当前目录为锚；只读 env，不碰盘）。
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), abs(&[session, a]));
        }
        Ok(crate::core::workspace::WorkRoots { shared: abs(&[session, "work"]), agents: map })
    }
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files.lock().expect("锁").insert(format!("{}/work/{}", session, name), bytes.to_vec());
        Ok(())
    }
    fn work_has(&self, session: &str, name: &str) -> bool {
        self.files.lock().expect("锁").contains_key(&format!("{}/work/{}", session, name))
    }
    fn list(&self, session: &str, agents: &[String]) -> Result<crate::core::workspace::WorkFiles, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 内存实现没有目录树，直接按前缀扫键（相对路径原样返回，/ 分隔）。
        let files = self.files.lock().expect("锁");
        let pick = |prefix: &str| -> Vec<String> {
            files
                .keys()
                .filter_map(|k| k.strip_prefix(prefix).map(|s| s.to_string()))
                .collect::<Vec<_>>()
        };
        let mut work = pick(&format!("{}/work/", session));
        work.sort();
        let mut map = BTreeMap::new();
        for a in agents {
            let mut got = pick(&format!("{}/{}/", session, a));
            got.sort();
            map.insert(a.clone(), got);
        }
        Ok(crate::core::workspace::WorkFiles { work, agents: map })
    }
}

/// 内存文件系统：内置文件工具的读写落在这里（测试可断言内容与越界拒绝）。
#[derive(Default)]
pub(crate) struct InMemorySysIo {
    files: Mutex<BTreeMap<String, String>>,
    fail: Option<String>,
}

impl InMemorySysIo {
    pub(crate) fn new() -> InMemorySysIo {
        InMemorySysIo { files: Mutex::new(BTreeMap::new()), fail: None }
    }

    /// 注入失败：read / write 一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemorySysIo {
        self.fail = Some(msg.to_string());
        self
    }
    pub(crate) fn seed(&self, parts: &[&str], text: &str) {
        self.files.lock().expect("锁").insert(p(parts), text.to_string());
    }
    pub(crate) fn get(&self, parts: &[&str]) -> Option<String> {
        self.files.lock().expect("锁").get(&p(parts)).cloned()
    }
}

impl SysIo for InMemorySysIo {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let key = path.to_string_lossy().into_owned();
        let text = self.files.lock().expect("锁").get(&key).cloned().ok_or_else(|| format!("读取失败：{} 不存在", key))?;
        Ok(FileRead { bytes: text.len(), text, lossy: false, cut: false })
    }
    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files.lock().expect("锁").insert(path.to_string_lossy().into_owned(), content.to_string());
        Ok(())
    }
}

/// 测试用绝对根：以当前工作目录为锚（只读 env，不碰盘）——真实路径模型下所有根都是绝对路径。
pub(crate) fn abs(parts: &[&str]) -> PathBuf {
    let mut b = std::env::current_dir().expect("取当前目录");
    for x in parts {
        b.push(x);
    }
    b
}

/// 绝对路径的字符串期望值（平台分隔符；用于 PathBuf / 内存 IO 的键）。
pub(crate) fn p(parts: &[&str]) -> String {
    abs(parts).to_string_lossy().into_owned()
}

/// 路径的**书写形式**（进 JSON / 提示词 / 转录都是它）：一律 / 分隔（Windows 反斜杠在 JSON 里非法）。
pub(crate) fn s(parts: &[&str]) -> String {
    p(parts).replace(std::path::MAIN_SEPARATOR, "/")
}

/// 测试沙箱：work 共享区 + agent 私有区 + 指定模块目录（都是绝对路径）。
pub(crate) fn test_sandbox(agent: &str, modules: &[&str]) -> crate::core::workspace::Sandbox {
    let mut map = BTreeMap::new();
    for id in modules {
        map.insert(id.to_string(), abs(&["mods", id]));
    }
    crate::core::workspace::Sandbox {
        work_name: "demo".to_string(),
        agent: agent.to_string(),
        shared: abs(&["demo", "work"]),
        private: abs(&["demo", agent]),
        modules: map,
        texts: test_prompts().core.tool_texts,
    }
}

/// 内存会话历史：供测试断言落盘与回放。
pub(crate) struct InMemoryHistory {
    metas: Mutex<BTreeMap<String, SessionMeta>>,
    events: Mutex<BTreeMap<String, Vec<serde_json::Value>>>,
    fail: Option<String>,
}

impl InMemoryHistory {
    pub(crate) fn new() -> InMemoryHistory {
        InMemoryHistory { metas: Mutex::new(BTreeMap::new()), events: Mutex::new(BTreeMap::new()), fail: None }
    }

    /// 注入失败：全部 HistoryStore 方法一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryHistory {
        self.fail = Some(msg.to_string());
        self
    }

    /// 失败注入的入口判定：Ok = 未注入。
    fn guard(&self) -> Result<(), String> {
        match &self.fail {
            Some(m) => Err(m.clone()),
            None => Ok(()),
        }
    }
}

impl HistoryStore for InMemoryHistory {
    fn create(&self, meta: &SessionMeta) -> Result<(), String> {
        self.guard()?;
        self.metas.lock().expect("锁").insert(meta.name.clone(), meta.clone());
        Ok(())
    }
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        self.guard()?;
        self.metas.lock().expect("锁").insert(meta.name.clone(), meta.clone());
        Ok(())
    }
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.guard()?;
        self.events.lock().expect("锁").entry(name.to_string()).or_default().extend_from_slice(events);
        Ok(())
    }
    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.guard()?;
        let metas = self.metas.lock().expect("锁");
        let events = self.events.lock().expect("锁");
        Ok(metas
            .values()
            .map(|m| HistoryView {
                name: m.name.clone(),
                mode: m.mode.clone(),
                ts: m.ts,
                done: events
                    .get(&m.name)
                    .map(|v| v.iter().any(|e| e.get("type").and_then(|t| t.as_str()) == Some("ended")))
                    .unwrap_or(false),
            })
            .collect())
    }
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.guard()?;
        let meta = self.metas.lock().expect("锁").get(name).cloned().ok_or_else(|| format!("无此会话：{}", name))?;
        let events = self.events.lock().expect("锁").get(name).cloned().unwrap_or_default();
        Ok((meta, events))
    }
    fn delete(&self, name: &str) -> Result<bool, String> {
        self.guard()?;
        let removed = self.metas.lock().expect("锁").remove(name).is_some();
        self.events.lock().expect("锁").remove(name);
        Ok(removed)
    }
}

/// 内存模型目录：回放固定模型名，并记录收到的 Provider（断言编辑期密钥复用）。
pub(crate) struct FakeCatalog {
    models: Vec<String>,
    pub(crate) seen: Mutex<Vec<Provider>>,
    fail: Option<String>,
}

impl FakeCatalog {
    pub(crate) fn new(models: Vec<String>) -> FakeCatalog {
        FakeCatalog { models, seen: Mutex::new(Vec::new()), fail: None }
    }

    /// 注入失败：list_models 返回该原因（失败注入不记录调用——错误路径没走到"用哪个通道"）。
    pub(crate) fn fail_with(mut self, msg: &str) -> FakeCatalog {
        self.fail = Some(msg.to_string());
        self
    }
}

impl ModelCatalog for FakeCatalog {
    fn list_models(&self, provider: &Provider) -> Result<Vec<String>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.seen.lock().expect("锁").push(provider.clone());
        Ok(self.models.clone())
    }
}

pub(crate) struct VecSource(pub(crate) Vec<Module>);

impl ModuleSource for VecSource {
    fn scan(&self) -> crate::core::module::Roster {
        crate::core::module::Roster { modules: self.0.clone(), rejected: Vec::new() }
    }
}

/// 无声围栏端口：测试里不碰任何 ACL（真实实现在 adapters/confine）。
pub(crate) struct NoFenceHost;
impl crate::core::ports::FenceHost for NoFenceHost {
    fn release(&self, _spec: &crate::core::fence::FenceSpec) -> Result<(), String> {
        Ok(())
    }
}

/// 记录型围栏端口：断言「删除会话时真的请求了撤销」。
pub(crate) struct RecordingFence {
    pub(crate) released: Mutex<Vec<String>>,
    fail: Option<String>,
}
impl RecordingFence {
    pub(crate) fn new() -> RecordingFence {
        RecordingFence { released: Mutex::new(Vec::new()), fail: None }
    }
    /// 注入失败：release 返回该原因（撤销失败必须如实传播，不能被当成已撤销）。
    pub(crate) fn fail_with(mut self, msg: &str) -> RecordingFence {
        self.fail = Some(msg.to_string());
        self
    }
}
impl crate::core::ports::FenceHost for RecordingFence {
    fn release(&self, spec: &crate::core::fence::FenceSpec) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.released.lock().expect("锁").push(spec.agent.clone());
        Ok(())
    }
}

/// 内存运行包库（测试组合根）：直接给出包清单，不碰盘。
pub(crate) struct InMemoryPackages(Vec<PackageManifest>);

impl InMemoryPackages {
    pub(crate) fn empty() -> InMemoryPackages {
        InMemoryPackages(Vec::new())
    }
    /// 用 yaml 文本造包（顺带覆盖清单解析）。
    pub(crate) fn with(yamls: &[&str]) -> InMemoryPackages {
        InMemoryPackages(yamls.iter().map(|y| pkg_yaml(y)).collect())
    }
}

impl PackageSource for InMemoryPackages {
    fn scan(&self) -> Library {
        Library::build(self.0.clone(), Vec::new())
    }
    fn dir(&self) -> PathBuf {
        abs(&["runtimes"])
    }
}

/// 用 yaml 造一份包清单（顺带覆盖清单解析）。
pub(crate) fn pkg_yaml(y: &str) -> PackageManifest {
    serde_yaml::from_str(y).expect("包清单必须能解析")
}

/// 造一个 prefix 类包（独立前缀 opt/rt/&lt;id&gt;-&lt;version&gt;）。
pub(crate) fn pkg(id: &str, version: &str) -> PackageManifest {
    pkg_yaml(&format!("id: {}
version: {}
prefix: opt/rt/{}-{}", id, version, id, version))
}

pub(crate) fn module_of(id: &str) -> Module {
    Module {
        manifest: ModuleManifest {
            id: id.to_string(),
            brief: format!("{} 的简介", id),
            system: format!("你负责{}", id),
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
        },
        root: abs(&[id]),
    }
}

/// 声明了运行能力的模块（工具的可用性按它判定）。
fn module_with_runtimes(id: &str, caps: &[&str]) -> Module {
    let mut m = module_of(id);
    m.manifest.runtimes = caps.iter().map(|s| s.to_string()).collect();
    m
}

/// 测试用静默 Live（不流式、不派发短暂事件）。
fn with_live<T>(f: impl FnOnce(&mut Live) -> T) -> T {
    let mut noop = |_e: SessionEvent| {};
    let mut live = Live { stream: false, cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)), emit: &mut noop };
    f(&mut live)
}

/// 把模块包成"每模块一个临时 agent"（协作的成员语义）。
fn agents_of(modules: &[&str]) -> Vec<AgentInstance> {
    modules
        .iter()
        .map(|m| AgentInstance { name: m.to_string(), transient: true, modules: vec![m.to_string()], model: None })
        .collect()
}

/// 测试用工作规格（模型留空 = 走核心默认 m）。
/// 单 agent 形态：1 个 agent（模块数不限；单模块时以模块名给 agent 起名，便于断言说话人）；
/// 协作形态：每模块一个 agent（成员 id = agent 名 = 模块名）。
fn work(name: &str, mode: WorkMode, modules: &[&str]) -> WorkSpec {
    let agents = if mode == WorkMode::Collab {
        agents_of(modules)
    } else {
        let agent_name = if modules.len() == 1 { modules[0] } else { "组合" };
        vec![AgentInstance {
            name: agent_name.to_string(),
            transient: true,
            modules: modules.iter().map(|s| s.to_string()).collect(),
            model: None,
        }]
    };
    WorkSpec { name: name.to_string(), mode, agents, task: None, delegate: false }
}

/// 协作工作规格（含需求）。
fn collab_work(name: &str, modules: &[&str], delegate: bool, task: &str) -> WorkSpec {
    let mut w = work(name, WorkMode::Collab, modules);
    w.delegate = delegate;
    w.task = Some(task.to_string());
    w
}

pub(crate) fn scripted(s: Vec<String>) -> BoxedChat {
    Box::new(FakeChat::new(s))
}

/// 共享脚本队列：多条核心响应按 complete 次序弹出（末条重复兜底）。
/// 与真实通道时序一致：建通道时不消费，调用时才消费。Arc 分身共享（网关与测试两侧）。
/// 共享脚本队列（Mutex 版：需跨线程 Send+Sync）。
pub(crate) struct SharedScript {
    pub(crate) q: Arc<Mutex<Vec<String>>>,
}

impl Chat for SharedScript {
    fn complete(&mut self, _messages: &[Msg], _stream: bool, _on: &mut dyn FnMut(Chunk) -> bool) -> String {
        let mut q = self.q.lock().expect("脚本队列锁");
        if q.len() > 1 {
            q.remove(0)
        } else {
            q.first().cloned().unwrap_or_default()
        }
    }
}

/// 脚本网关：按 agent 实例名回放各自脚本；核心通道走共享队列。
pub(crate) struct ScriptGateway {
    member: BTreeMap<String, Vec<String>>,
    core: Arc<Mutex<Vec<String>>>,
}
impl ScriptGateway {
    pub(crate) fn new(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
        ScriptGateway { member, core: Arc::new(Mutex::new(core)) }
    }
}

impl ChatGateway for ScriptGateway {
    fn member_channel(&self, _c: Option<&Channel>, id: &str) -> (BoxedChat, Option<String>) {
        let script = self.member.get(id).cloned().unwrap_or_else(|| {
            vec!["{\"type\":\"say\",\"text\":\"（演示）收到。\"}".to_string()]
        });
        (scripted(script), None)
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (Box::new(SharedScript { q: Arc::clone(&self.core) }), false)
    }
}

/// 提示词册替身：默认回放内置册子；fail_with 注入加载失败。
pub(crate) struct TestPrompts {
    fail: Option<String>,
}

impl TestPrompts {
    pub(crate) fn ok() -> TestPrompts {
        TestPrompts { fail: None }
    }
    pub(crate) fn fail_with(mut self, msg: &str) -> TestPrompts {
        self.fail = Some(msg.to_string());
        self
    }
}

impl PromptSource for TestPrompts {
    fn load(&self) -> Result<Prompts, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(test_prompts())
    }
}

pub(crate) fn test_prompts() -> Prompts {
    serde_yaml::from_str::<Prompts>(include_str!("../prompts.yaml")).expect("内置提示词册必须合法")
}

fn core_with(modules: Vec<Module>, gateway: ScriptGateway) -> Core {
    core_with_runner(modules, gateway, Arc::new(SilentRunner))
}

/// 注入指定内存工作区的装配（断言 @ 文件清单取自会话 meta.agents）。
fn core_with_workspace(modules: Vec<Module>, gateway: ScriptGateway, ws: Arc<InMemoryWorkspace>) -> Core {
    Core::new(
        Arc::new(InMemorySettings::new()),
        Arc::new(InMemoryHistory::new()),
        ws,
        Arc::new(VecSource(modules)),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

/// 注入指定内存文件系统的装配（断言内置文件工具真正落盘）。
fn core_with_io(modules: Vec<Module>, gateway: ScriptGateway, io: Arc<InMemorySysIo>) -> Core {
    core_with_all(
        modules,
        gateway,
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(InMemoryHistory::new()),
        io,
    )
}

fn core_with_runner(modules: Vec<Module>, gateway: ScriptGateway, runner: Arc<impl ToolRunner + Send + Sync + 'static>) -> Core {
    core_with_catalog(modules, gateway, runner, Arc::new(FakeCatalog::new(vec!["m".to_string()])))
}

/// 指定模型目录的装配（断言编辑期密钥复用）。
fn core_with_catalog(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
    catalog: Arc<FakeCatalog>,
) -> Core {
    core_with_all(
        modules,
        gateway,
        runner,
        catalog,
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemorySysIo::new()),
    )
}

/// 指定全部端口的装配（历史落盘断言用）：默认空包库。
fn core_with_all(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Core {
    core_with_pkgs(modules, gateway, runner, catalog, history, io, Arc::new(InMemoryPackages::empty()))
}

/// 指定全部端口 + 运行包库的装配（断言缺包诊断与工具回执）。
fn core_with_pkgs(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
    packages: Arc<InMemoryPackages>,
) -> Core {
    Core::new(
        Arc::new(InMemorySettings::new()),
        history,
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(modules)),
        packages,
        Arc::new(NoFenceHost),
        Arc::new(gateway),
        catalog,
        runner,
        io,
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

fn gw(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
    ScriptGateway { member, core: Arc::new(Mutex::new(core)) }
}

// ---------- 信封 ----------

#[test]
fn envelope_parse_clean() {
    let r = crate::core::envelope::parse("{\"type\":\"ask\",\"text\":\"要 A 还是 B？\"}");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Ask));
    assert_eq!(r.text, "要 A 还是 B？");
    assert!(!r.degraded);
}

#[test]
fn envelope_degraded_keeps_raw() {
    let r = crate::core::envelope::parse("这不是 JSON");
    assert!(r.degraded);
    assert_eq!(r.text, "这不是 JSON");
}

#[test]
fn envelope_wrapped_json_still_parses() {
    let r = crate::core::envelope::parse("好的：{\"type\":\"agree\",\"text\":\"同意\"} 以上。");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Agree));
    assert!(!r.degraded);
}

#[test]
fn envelope_text_may_be_omitted() {
    let r = crate::core::envelope::parse("{\"type\":\"agree\"}");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Agree), "缺 text 不影响表态");
    assert_eq!(r.text, "", "缺 text = 空串");
    assert!(!r.degraded);
    let say = crate::core::envelope::parse("{\"type\":\"say\"}");
    assert!(matches!(say.verb, crate::core::envelope::Verb::Say) && !say.degraded, "缺 text 的发言仍是干净信封");
    assert!(say.text.is_empty());
    // 缺 name 的工具信封仍是 malformed 信号，不被缺省 text 收编成发言。
    let bad = crate::core::envelope::parse("{\"type\":\"tool\",\"args\":{}}");
    assert!(matches!(bad.verb, crate::core::envelope::Verb::Tool));
    assert!(bad.tool.map(|t| t.malformed).unwrap_or(false));
}

// ---------- 提示词渲染层 ----------

#[test]
fn prompt_render_replaces_and_rejects_missing() {
    let ok = render("你好 {{name}}！", &[("name", "世界".to_string())]).unwrap();
    assert_eq!(ok, "你好 世界！");
    assert!(render("{{missing}}", &[]).is_err());
}

#[test]
fn prompt_book_loads_from_yaml() {
    let p = test_prompts();
    assert!(p.core.chat_protocol.contains("ask"));
    assert!(p.core.discuss.opener.contains("{{protocol}}"));
}

#[test]
fn prompt_render_keeps_single_braces() {
    let ok = render("输出 {\"a\":1} 和 {{x}}", &[("x", "Y".to_string())]).unwrap();
    assert_eq!(ok, "输出 {\"a\":1} 和 Y");
}

// ---------- 登记处解析链与密钥治理 ----------

#[test]
fn settings_resolves_model_to_channel() {
    let mut s = Settings::default();
    s.providers.insert("p".into(), Provider { base_url: "http://x".into(), api_key: "k".into() });
    s.models.insert("m".into(), ModelEntry { name: "展示名".into(), api_model: "real-model".into(), provider: "p".into(), note: String::new() });
    s.core = Some("m".into());
    let ch = s.resolve("m").unwrap();
    assert_eq!(ch.model, "real-model", "发给供应商的是 api_model，不是展示名");
    assert_eq!(ch.provider.base_url, "http://x");
    assert!(s.resolve("ghost").is_err(), "未知模型必须报错");
    assert_eq!(s.core_channel().unwrap().model, "real-model");
}

#[test]
fn provider_lifecycle_and_key_never_leaks_to_view() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.provider_upsert("p1", "http://x", "sk-密钥XYZ").unwrap();
    for line in core.provider_lines() {
        assert!(!line.contains("sk-密钥XYZ"), "视图出现密钥：{}", line);
    }
    for v in core.provider_views() {
        assert!(!format!("{:?}", v).contains("sk-密钥XYZ"));
    }
    // 仍被模型引用时拒绝删除供应商（不静默级联）
    core.model_upsert("m1", "M", "api-m", "p1", "快").unwrap();
    assert!(core.provider_remove("p1").unwrap_err().contains("仍被模型引用"));
    assert!(core.model_remove("m1").unwrap());
    assert!(core.provider_remove("p1").unwrap());
}

#[test]
fn model_guards_core_default_and_discovery_uses_stored_provider() {
    let catalog = Arc::new(FakeCatalog::new(vec!["m-a".to_string(), "m-b".to_string()]));
    let mut core = core_with_catalog(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::clone(&catalog),
    );
    core.provider_upsert("p2", "http://x", "k").unwrap();
    assert_eq!(core.discover_models("p2").unwrap(), vec!["m-a".to_string(), "m-b".to_string()]);
    assert_eq!(catalog.seen.lock().expect("锁")[0].base_url, "http://x");
    assert!(core.discover_models("ghost").unwrap_err().contains("无此供应商"));
    // 引用了不存在的供应商 → 拒绝登记
    assert!(core.model_upsert("bad", "B", "b", "ghost", "").unwrap_err().contains("无此供应商"));
    // 核心默认模型不可删；换默认后旧的可删
    assert!(core.model_remove("m").unwrap_err().contains("核心默认模型"));
    core.model_upsert("m2", "M2", "api-m2", "p2", "").unwrap();
    assert!(core.core_set_model("m2").unwrap());
    assert!(core.model_remove("m").unwrap());
}

#[test]
fn create_work_persists_and_history_replays() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core.create_work(work("工作一", WorkMode::Single, &["a"])).unwrap().sid;
    assert_eq!(sid, "工作一");
    with_live(|l| core.single_say(&sid, "你好", l)).unwrap();
    let list = core.history_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "工作一");
    assert_eq!(list[0].mode, "single");
    let (meta, events) = core.history_open("工作一").unwrap();
    assert_eq!(meta.mode, "single");
    assert!(events.iter().any(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript")), "历史里应有转录事件");
    // 重名（磁盘已存在）被拒
    assert!(core.create_work(work("工作一", WorkMode::Single, &["a"])).unwrap_err().contains("已存在"));
    // 删除会话：内存与磁盘一起删，之后名字可复用
    assert!(core.history_delete("工作一").unwrap());
    assert!(core.history_list().is_empty());
    assert!(core.create_work(work("工作一", WorkMode::Single, &["a"])).is_ok());
}

#[test]
fn direct_rewind_drops_tail_then_continue_allows_user_turn() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"答一\"}".to_string(),
            "{\"type\":\"say\",\"text\":\"答二\"}".to_string(),
            "{\"type\":\"say\",\"text\":\"续答\"}".to_string(),
        ],
    );
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap(); // 行 0=用户, 1=AI
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap(); // 行 2=用户, 3=AI

    // 末条是 AI → 继续被拦，只给提醒、不发请求
    let blocked = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(matches!(&blocked[0], SessionEvent::Notice(n) if n.contains("最后一条是 AI 发言")));

    // 回档到第 1 行：保留前 1 行（= 删第 1 行及其后），只留「用户·问一」
    let replayed = core.rewind(&sid, 1).unwrap();
    let lines = replay_lines(&replayed);
    assert_eq!(lines, vec!["[用户] 问一".to_string()]);

    // 末条是用户 → 继续直接续跑（不再需要用户发言）
    let ev = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    let reply = ev.iter().find_map(|e| match e {
        SessionEvent::Transcript(l) => l.first().map(|x| x.line.clone()),
        _ => None,
    }).expect("应有转录回复");
    assert!(reply.contains("续答"), "继续应直接用现有历史问模型：{}", reply);

    // 落盘历史也按回档截断回放
    let (_, events) = core.history_open("w").unwrap();
    assert_eq!(replay_lines(&events), vec!["[用户] 问一".to_string(), format!("[a] {}", "续答")]);
}

/// 从事件流里抽出转录行，便于断言。
fn replay_lines(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .flat_map(|e| e.get("lines").and_then(|l| l.as_array()).cloned().unwrap_or_default())
        .map(|l| l.get("line").and_then(|x| x.as_str()).unwrap_or("").to_string())
        .collect()
}

/// 事件流里的转录行视图：(id, line, 是否是 tool 行)。
fn transcript_rows(events: &[SessionEvent]) -> Vec<(u64, String, bool)> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls.iter().map(|l| (l.id, l.line.clone(), l.tool.is_some())).collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 事件流里的工具调用视图（tool 行携带的那个）。
fn tool_views(events: &[SessionEvent]) -> Vec<crate::core::events::ToolCallView> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls.iter().filter_map(|l| l.tool.clone()).collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 事件流里 tool 行的行文本（给人看的那一行）。
fn tool_line_texts(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => {
                ls.iter().filter(|l| l.tool.is_some()).map(|l| l.line.clone()).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn rewind_keeps_only_lines_before_the_mark() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"答一\"}".to_string(),
        "{\"type\":\"say\",\"text\":\"答二\"}".to_string(),
    ]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap(); // 行 0 用户 / 1 AI
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap(); // 行 2 用户 / 3 AI

    // 回档到第 2 行 = 删第 2 行及其后 → 只留前 2 行，历史与 marks 同步截断
    let replayed = core.rewind(&sid, 2).unwrap();
    assert_eq!(replay_lines(&replayed), vec!["[用户] 问一".to_string(), "[a] 答一".to_string()]);
    assert_eq!(core.single_history(&sid).unwrap().len(), 3, "system + 用户 + 答一");

    // 回档到第 0 行 = 转录清空、历史只剩 system
    let replayed = core.rewind(&sid, 0).unwrap();
    assert!(replay_lines(&replayed).is_empty(), "点第一行 → 转录清空：{:?}", replayed);
    assert_eq!(core.single_history(&sid).unwrap().len(), 1, "历史只剩 system");

    // next_line 归零：下一条行 id 从 0 开始
    let ev = with_live(|l| core.single_say(&sid, "再来", l)).unwrap();
    let first = transcript_rows(&ev).first().cloned().expect("应有新行");
    assert_eq!(first.0, 0, "回档清空后行 id 从头计");
}

#[test]
fn rebuilt_context_keeps_tool_result() {
    // 同一份落盘历史 + 新的 Core 模拟「重启」：重建上下文时工具子轮不能丢。
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["w", "a", "note.txt"]);
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        format!("{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"你好\"}}}}", note),
        "{\"type\":\"say\",\"text\":\"写完了\"}".to_string(),
    ]);
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    drop(core); // 会话随进程消失，只剩落盘流水

    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    // 内存里没有这个会话 → 走 rebuild_session（保留全部 3 行）
    core2.rewind(&sid, 3).unwrap();
    let h = core2.single_history(&sid).expect("重建后应在内存里");
    assert!(
        h.iter().any(|m| m.role == "assistant" && m.content.contains("\"type\":\"tool\"")),
        "重建要还原模型原始工具请求：{:?}",
        h
    );
    assert!(
        h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "重建必须保留工具结果（这条正是要修的 bug）：{:?}",
        h
    );
    assert_eq!(io.get(&["w", "a", "note.txt"]).as_deref(), Some("你好"), "重建后沙箱里的成品仍在");
    assert!(h.iter().any(|m| m.role == "assistant" && m.content.contains("写完了")));
}

#[test]
fn collab_state_derive_and_withdraw() {
    use crate::core::collab_state::derive;
    let ev = |id: u64, line: &str| serde_json::json!({"type":"transcript","lines":[{"id":id,"line":line}]});
    let events = vec![
        ev(0, "[用户:需求] 做个东西"),
        ev(1, "[用户:开始] yes"),
        ev(2, "[轮次 2]"),
        ev(3, "[a:agree] 同意"),
    ];
    let st = derive(&events, &["a".to_string()]);
    assert_eq!(st.task.as_deref(), Some("做个东西"));
    assert!(st.begun && !st.allow);
    assert_eq!(st.round, 2);
    assert!(st.agreed["a"] && st.closed, "全员同意即收敛");
    // 撤回该同意后：不再算同意、讨论不再收敛
    let mut withdrawn = events.clone();
    withdrawn.push(ev(4, "[用户:撤回] a"));
    let st2 = derive(&withdrawn, &["a".to_string()]);
    assert!(!st2.agreed["a"] && !st2.closed);
    // 代拟行只给人看：名单不由它派生（权威来源是 meta.agents）。
    let with_slate = vec![ev(0, "[代拟] 甲〈a〉→ m（对口）；乙（复用；补位）"), ev(1, "[用户:名单] 确认")];
    let st3 = derive(&with_slate, &["a".to_string()]);
    assert_eq!(st3.picked, vec!["a".to_string()], "名单仍来自 meta.agents");
    assert!(st3.slate.is_some() && st3.slate_confirmed, "只保留原文供展示");
}

#[test]
fn collab_rewind_rebuilds_from_transcript_and_resume_waits_at_gate() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["{\"type\":\"say\",\"text\":\"好\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(collab_work("c", &["a"], false, "做个东西")).unwrap().sid;
    assert!(matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))));
    // 回档到需求行之后（保留前 1 行 = 需求行）：协作走「按转录重建」
    let replayed = core.rewind(&sid, 1).unwrap();
    assert_eq!(replay_lines(&replayed), vec!["[用户:需求] 做个东西".to_string()]);
    assert!(matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))), "重建后仍等确认开始");
    // 继续：未开始不由继续代劳，只提醒
    let ev = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(matches!(&ev[0], SessionEvent::Notice(n) if n.contains("裁决门")));
}

#[test]
fn collab_update_task_rewinds_and_latest_wins() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core.create_work(collab_work("c", &["a"], false, "旧需求")).unwrap().sid;
    let replayed = core.update_task(&sid, "新需求").unwrap();
    assert_eq!(replay_lines(&replayed), vec!["[用户:需求] 旧需求".to_string(), "[用户:需求] 新需求".to_string()]);
    // 派生以最后一条需求为准
    let (_, events) = core.history_open("c").unwrap();
    let st = crate::core::collab_state::derive(&events, &["a".to_string()]);
    assert_eq!(st.task.as_deref(), Some("新需求"));
}

#[test]
fn suggest_models_recommends_agents() {
    // 不存在的模块 / 不存在的模型 / 重复占用的模块一律拒收。
    let script = "{\"agents\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"},{\"name\":\"乙\",\"modules\":[\"b\"],\"model\":\"m\",\"why\":\"补位\"},{\"name\":\"鬼\",\"modules\":[\"ghost\"],\"model\":\"m\",\"why\":\"模块不存在\"},{\"name\":\"丙\",\"modules\":[\"a\"],\"model\":\"nope\",\"why\":\"模型不存在\"},{\"name\":\"丁\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"重复占模块\"}]}".to_string();
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec![script.clone()]));
    // 协作：两个独立 agent，各带自己的模块与模型。
    let collab = core.suggest_models("做个东西", WorkMode::Collab).unwrap();
    assert_eq!(collab.len(), 2);
    assert_eq!(collab[0].name, "甲");
    assert_eq!(collab[0].modules, vec!["a".to_string()]);
    assert_eq!(collab[1].modules, vec!["b".to_string()]);
    assert_eq!(collab[0].model, "m");
    assert_eq!(collab[0].why, "对口");
    // 单 agent：多条推荐 → 并成一个临时 agent（并过的不是任何单个已存 agent，故 reuse=false）。
    let merged = core.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].modules, vec!["a".to_string(), "b".to_string()]);
    assert!(!merged[0].reuse, "并出来的 agent 不是复用项");
}

#[test]
fn suggest_models_single_mode_keeps_lone_pick_as_is() {
    // 只给一条 → 原样采纳（模块数与 reuse 都保持它自己的，不裁模块、不改 reuse）。
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec![
        "{\"agents\":[{\"name\":\"全能\",\"modules\":[\"a\",\"b\"],\"model\":\"m\",\"why\":\"一个 AI 全包\"}]}".to_string(),
    ]));
    let out = core.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, "全能");
    assert_eq!(out[0].modules, vec!["a".to_string(), "b".to_string()], "单 agent 不再被裁到一个模块");
    assert!(!out[0].reuse);
    // 复用项也要原样保留 reuse 标记
    let mut reuse = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"agents\":[{\"agent\":\"调研\",\"why\":\"正好\"}]}".to_string(),
    ]));
    reuse.agent_upsert("调研", &["a".to_string()], "m", "").unwrap();
    let got = reuse.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].reuse, "一条复用项必须保留 reuse");
    assert_eq!(got[0].model, "m");
}

#[test]
fn suggest_models_reuses_stored_agent_without_suggesting_model() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        // 只有复用项（模型与模块都取登记处自己的）；幽灵项应被拒收。
        "{\"agents\":[{\"agent\":\"调研\",\"why\":\"正好用得上\"},{\"agent\":\"幽灵\",\"why\":\"不在登记处\"}]}".to_string(),
    ]));
    core.agent_upsert("调研", &["a".to_string()], "m", "说明").unwrap();
    let out = core.suggest_models("做个东西", WorkMode::Collab).unwrap();
    assert_eq!(out.len(), 1, "非法的复用项必须被拒收");
    assert!(out[0].reuse, "复用项要如实标记");
    assert_eq!(out[0].name, "调研");
    assert_eq!(out[0].modules, vec!["a".to_string()]);
    assert_eq!(out[0].model, "m", "模型取登记处里那个，核心不代拟");
    // 全部拒收 → 明确报错（不悄悄给个空名单）
    let mut empty = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"agents\":[{\"agent\":\"幽灵\",\"why\":\"不在登记处\"}]}".to_string(),
    ]));
    empty.agent_upsert("调研", &["a".to_string()], "m", "").unwrap();
    assert!(empty.suggest_models("做个东西", WorkMode::Collab).unwrap_err().contains("没有可用结果"));
}

#[test]
fn same_named_tools_across_modules_are_no_longer_a_conflict() {
    let mut a = module_of("a");
    a.manifest.tools.insert("dump".to_string(), "python a/dump.py".to_string());
    let mut b = module_of("b");
    b.manifest.tools.insert("dump".to_string(), "python b/dump.py".to_string());
    let mut core = core_with(vec![a, b], gw(BTreeMap::new(), vec!["[]".into()]));
    // 跨模块同名工具不再冲突：信封里的 module 消歧（行为见 same_named_tools_in_two_modules_run_in_their_own_root）。
    let one_agent = WorkSpec {
        name: "w".to_string(),
        mode: WorkMode::Single,
        agents: vec![AgentInstance { name: "组合".to_string(), transient: true, modules: vec!["a".to_string(), "b".to_string()], model: None }],
        task: None,
        delegate: false,
    };
    assert!(core.create_work(one_agent).is_ok(), "同名工具同属一个 agent 也应能建");
    // 仍然保留的校验：同一模块不得同时属于两个 agent（沙箱与发言归属会歧义）。
    let cross = WorkSpec {
        name: "cross".to_string(),
        mode: WorkMode::Collab,
        agents: vec![
            AgentInstance { name: "甲".to_string(), transient: true, modules: vec!["a".to_string()], model: None },
            AgentInstance { name: "乙".to_string(), transient: true, modules: vec!["a".to_string()], model: None },
        ],
        task: Some("需求".to_string()),
        delegate: false,
    };
    assert!(core.create_work(cross).unwrap_err().contains("被多个 agent"));
}

#[test]
fn agent_crud_and_work_with_agents() {
    let mut core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec!["[]".into()]));
    assert!(core.agent_upsert("", &["a".to_string()], "", "").unwrap_err().contains("不能为空"));
    assert!(core.agent_upsert("x", &[], "", "").unwrap_err().contains("至少要有一个模块"));
    assert!(core.agent_upsert("x", &["ghost".to_string()], "", "").unwrap_err().contains("无此模块"));
    assert!(core.agent_upsert("x", &["a".to_string()], "nope", "").unwrap_err().contains("无此模型"));
    core.agent_upsert("调研", &["a".to_string(), "b".to_string()], "m", "说明").unwrap();
    assert_eq!(core.agent_views().len(), 1);
    assert_eq!(core.agent_views()[0].modules.len(), 2);

    // 组合：一个 agent（多模块合并）
    let spec = WorkSpec {
        name: "w".to_string(),
        mode: WorkMode::Single,
        agents: vec![AgentInstance { name: "调研".to_string(), transient: false, modules: vec!["a".to_string(), "b".to_string()], model: Some("m".to_string()) }],
        task: None,
        delegate: false,
    };
    let opened = core.create_work(spec).unwrap();
    assert_eq!(opened.agents, vec!["调研".to_string()]);
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研");

    // 协作：多 agent；同工作内重名自动加尾号
    let mut dup = collab_work("c", &["a", "b"], false, "需求");
    dup.agents[1].name = dup.agents[0].name.clone();
    let o = core.create_work(dup).unwrap();
    assert_eq!(o.agents, vec!["a".to_string(), "a-2".to_string()]);

    // 同一模块不得跨 agent 重复
    let cross = WorkSpec {
        name: "cross".to_string(),
        mode: WorkMode::Collab,
        agents: vec![
            AgentInstance { name: "x".to_string(), transient: true, modules: vec!["a".to_string()], model: None },
            AgentInstance { name: "y".to_string(), transient: true, modules: vec!["a".to_string()], model: None },
        ],
        task: Some("需求".to_string()),
        delegate: false,
    };
    assert!(core.create_work(cross).unwrap_err().contains("被多个 agent"));

    assert!(core.agent_remove("调研").unwrap());
    assert!(core.agent_views().is_empty());
}

#[test]
fn work_upload_conflict_and_safe_name() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core.create_work(work("up", WorkMode::Single, &["a"])).unwrap().sid;
    assert!(core.work_upload(&sid, "x.txt", b"hello", false).unwrap());
    assert!(!core.work_upload(&sid, "x.txt", b"again", false).unwrap(), "同名不覆盖");
    assert!(core.work_upload(&sid, "x.txt", b"again", true).unwrap(), "显式覆盖");
    assert!(core.work_upload("ghost", "x.txt", b"x", false).unwrap_err().contains("无此会话"));
    assert!(core.work_upload(&sid, "../evil.txt", b"x", false).unwrap_err().contains("路径分隔符"));
}

#[test]
fn create_work_validates_user_choices() {
    let mut core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec!["[]".into()]));
    assert!(core.create_work(work("", WorkMode::Single, &["a"])).unwrap_err().contains("工作名不能为空"));
    assert!(core.create_work(work("x", WorkMode::Single, &["ghost"])).unwrap_err().contains("无此模块"));
    assert!(core.create_work(work("x", WorkMode::Single, &[])).unwrap_err().contains("至少要有一个模块"));
    // 单 agent 形态只接受一个 agent（模块数不限，多模块合法，见 single_mode_accepts_multi_module_agent）
    let two_agents = WorkSpec {
        name: "x".to_string(),
        mode: WorkMode::Single,
        agents: vec![
            AgentInstance { name: "甲".to_string(), transient: true, modules: vec!["a".to_string()], model: None },
            AgentInstance { name: "乙".to_string(), transient: true, modules: vec!["b".to_string()], model: None },
        ],
        task: None,
        delegate: false,
    };
    assert!(core.create_work(two_agents).unwrap_err().contains("只接受一个 agent"));
    assert!(core.create_work(collab_work("x", &["a"], false, "  ")).unwrap_err().contains("必须填写本次需求"));
    let mut bad_model = work("x", WorkMode::Single, &["a"]);
    bad_model.agents[0].model = Some("ghost".to_string());
    assert!(core.create_work(bad_model).unwrap_err().contains("无此模型"));
    // 建一次成功后重名被拒
    assert!(core.create_work(work("同名", WorkMode::Single, &["a"])).is_ok());
    assert!(core.create_work(work("同名", WorkMode::Single, &["a"])).unwrap_err().contains("已存在"));
    // 非法字符（将来要当目录名）
    assert!(core.create_work(work("a/b", WorkMode::Single, &["a"])).unwrap_err().contains("不能包含"));
}

#[test]
fn model_catalog_parses_openai_shape_and_rejects_bad() {
    use crate::adapters::model_catalog::parse_models;
    assert_eq!(
        parse_models(r#"{"data":[{"id":"gpt-4o"},{"id":"o3"},{"id":"gpt-4o"}]}"#).unwrap(),
        vec!["gpt-4o".to_string(), "o3".to_string()]
    );
    assert!(parse_models(r#"{"models":["a"]}"#).is_err(), "缺 data 必须报错");
    assert!(parse_models(r#"{"data":[]}"#).is_err(), "空列表必须报错");
    assert!(parse_models("不是 JSON").is_err());
}

#[test]
fn endpoint_candidates_complete_and_fall_back() {
    use crate::adapters::endpoint::{chat_candidates, models_candidates, retryable_status};
    // 已带版本段：只补后缀（含尾斜杠）
    assert_eq!(chat_candidates("https://api.x/v1"), vec!["https://api.x/v1/chat/completions"]);
    assert_eq!(chat_candidates("https://api.x/v1/"), vec!["https://api.x/v1/chat/completions"]);
    assert_eq!(chat_candidates("https://api.x/v2"), vec!["https://api.x/v2/chat/completions"]);
    assert_eq!(models_candidates("https://api.x/v1"), vec!["https://api.x/v1/models"]);
    // 无版本段：先直连，连不上再回落 /v1
    assert_eq!(
        chat_candidates("https://api.x"),
        vec!["https://api.x/chat/completions", "https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        models_candidates("https://api.x"),
        vec!["https://api.x/models", "https://api.x/v1/models"]
    );
    // 已是完整端点：原样，防重复拼接
    assert_eq!(chat_candidates("https://api.x/v1/chat/completions"), vec!["https://api.x/v1/chat/completions"]);
    assert_eq!(models_candidates("https://api.x/v1/models"), vec!["https://api.x/v1/models"]);
    // 换候选判定：只有 404/405 换，鉴权类立即报
    assert!(retryable_status(404) && retryable_status(405));
    assert!(!retryable_status(401) && !retryable_status(403) && !retryable_status(429) && !retryable_status(500));
}

#[test]
fn endpoint_resolve_candidates_retries_shape_mismatch_and_stops_on_fatal() {
    use crate::adapters::endpoint::{resolve_candidates, Attempt};
    let cands = vec!["a".to_string(), "b".to_string(), "c".to_string()];

    // 回归：SPA catch-all 返回 200 HTML（形状不符=Retry）时必须换到下一个候选，而不是立即报错。
    let mut seen = Vec::new();
    let got = resolve_candidates(
        &cands,
        |url| {
            seen.push(url.to_string());
            if url == "b" { Attempt::Ok(7) } else { Attempt::Retry("响应不是 JSON".to_string()) }
        },
        |_, _, _| {},
    );
    assert_eq!(got.unwrap(), ("b".to_string(), 7));
    assert_eq!(seen, vec!["a".to_string(), "b".to_string()]);

    // 鉴权类 Fatal 立即停，不再试后续候选。
    let mut seen2 = Vec::new();
    let fatal: Result<(String, i32), String> = resolve_candidates(
        &cands,
        |url| {
            seen2.push(url.to_string());
            Attempt::Fatal("供应商返回 401".to_string())
        },
        |_, _, _| {},
    );
    assert_eq!(fatal.unwrap_err(), "供应商返回 401");
    assert_eq!(seen2, vec!["a".to_string()]);

    // 全部 Retry 耗尽：报最后一个错误，不静默。
    let all: Result<(String, i32), String> = resolve_candidates(&cands, |_| Attempt::Retry("失败".to_string()), |_, _, _| {});
    assert_eq!(all.unwrap_err(), "失败");
}

// ---------- 模块清单（内存来源） ----------

#[test]
fn roster_lists_modules() {
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec!["[]".into()]));
    let r = core.scan();
    let ids: Vec<_> = r.modules.iter().map(|m| m.manifest.id.clone()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}

// ---------- 讨论引擎 ----------

fn scripted_discussion(scripts: Vec<Vec<String>>, allow: bool) -> Discussion {
    let members: Vec<Member> = scripts
        .into_iter()
        .enumerate()
        .map(|(i, s)| Member::new(&format!("m{}", i), format!("职责{}", i), scripted(s)))
        .collect();
    Discussion::new(members, allow, test_prompts())
}

#[test]
fn discussion_full_agreement() {
    let mut d = scripted_discussion(
        vec![
            vec!["{\"type\":\"say\",\"text\":\"好\"}".into(), "{\"type\":\"agree\",\"text\":\"同意\"}".into()],
            vec!["{\"type\":\"say\",\"text\":\"行\"}".into(), "{\"type\":\"agree\",\"text\":\"同意\"}".into()],
        ],
        false,
    );
    d.open("任务");
    loop {
        match d.step() {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.text.contains("[m0:agree]")));
}

#[test]
fn discussion_ask_pauses() {
    let mut d = scripted_discussion(vec![vec!["{\"type\":\"ask\",\"text\":\"需要参数?\"}".into()]], false);
    d.open("任务");
    match d.step() {
        TurnOut::AskUser { member, question } => {
            assert_eq!(member, "m0");
            assert_eq!(question, "需要参数?");
        }
        _ => panic!("应暂停请教"),
    }
}

#[test]
fn discussion_leave_shrinks() {
    let mut d = scripted_discussion(vec![vec!["{\"type\":\"leave\",\"text\":\"撤了\"}".into()]], false);
    d.open("任务");
    let _ = d.step();
    assert!(d.members.iter().all(|m| !m.present));
}

#[test]
fn discussion_autonomy_archives_ask() {
    let mut d = scripted_discussion(
        vec![vec![
            "{\"type\":\"say\",\"text\":\"开场\"}".into(),
            "{\"type\":\"ask\",\"text\":\"细节?\"}".into(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
        ]],
        true,
    );
    d.open("任务");
    loop {
        match d.step() {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("自裁模式不该暂停"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.text.contains("自裁")));
}

#[test]
fn discussion_round_cap_enforced() {
    let mut d = scripted_discussion(vec![vec!["{\"type\":\"say\",\"text\":\"继续\"}".into()]; 2], false);
    d.open("任务");
    loop {
        match d.step() {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
        }
    }
    assert!(d.round > MAX_ROUNDS);
}

#[test]
fn degraded_discussion_line_carries_a_structured_flag() {
    let prompts = test_prompts();
    // 成员给出"不是信封"的原文 → 该行按降级收录：文本里有说明，**结构上另带 degraded**。
    let members = vec![Member::new("m0", "职责".to_string(), scripted(vec!["我觉得可以".into()]))];
    let mut disc = Discussion::new(members, true, prompts.clone());
    disc.open("任务");
    let line = disc.transcript.iter().find(|l| l.text.starts_with("[m0:say]")).expect("应有 m0 的发言行");
    assert!(line.degraded, "降级必须带结构化标记（呈现层靠它，不靠匹配文案）");
    assert!(
        line.text.contains(&prompts.core.tool_texts.discuss_degraded),
        "文本里仍保留给人/模型看的说明"
    );

    // 线格式：只在为真时写出 degraded
    let yes = SessionEvent::Transcript(vec![crate::core::events::LineView {
        id: 0,
        line: "x".into(),
        degraded: true,
        ..Default::default()
    }])
    .to_json();
    assert_eq!(yes["lines"][0]["degraded"], serde_json::Value::Bool(true));
    let no = SessionEvent::Transcript(vec![crate::core::events::LineView { id: 0, line: "x".into(), ..Default::default() }]).to_json();
    assert!(no["lines"][0].get("degraded").is_none(), "非降级行不写这个字段");
}

// ---------- 执行/验收 ----------

#[test]
fn execution_review_pass_and_fail_paths() {
    let prompts = test_prompts();
    let mut members = vec![Member::new("m0", "职责".to_string(), scripted(vec!["{\"type\":\"say\",\"text\":\"汇报内容\"}".into()]))];
    let mut exec = crate::core::engine::Execution::run(members.as_mut_slice(), "任务A", &prompts);
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("汇报内容"));
    let mut core_chat = scripted(vec!["[{\"item\":\"A\",\"status\":\"fail\",\"reason\":\"没做完\"}]".into()]);
    exec.review(core_chat.as_mut(), "方案", &prompts);
    assert!(!exec.all_pass());
    exec.rerun(members.as_mut_slice(), "任务A", "- A：没做完", &prompts);
    let mut core_chat2 = scripted(vec!["[{\"item\":\"A\",\"status\":\"pass\"}]".into()]);
    exec.review(core_chat2.as_mut(), "方案", &prompts);
    assert!(exec.all_pass());
}

#[test]
fn review_parse_failure_is_conservative_fail() {
    let prompts = test_prompts();
    let mut members = vec![Member::new("m0", "职责".to_string(), scripted(vec!["{\"type\":\"say\",\"text\":\"x\"}".into()]))];
    let mut exec = crate::core::engine::Execution::run(members.as_mut_slice(), "任务", &prompts);
    let mut core_chat = scripted(vec!["完全不是清单".to_string()]);
    exec.review(core_chat.as_mut(), "方案", &prompts);
    assert!(exec.items.is_empty());
    assert!(!exec.all_pass(), "解析失败必须保守判否");
}

// ---------- Core 门面：会话中心（内存组合根） ----------

#[test]
fn core_direct_seeds_system_prompt() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["{\"type\":\"say\",\"text\":\"你好\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    // 回归：单 agent 会话的历史首条必须是职责提示词（system）。
    let h = core.single_history(&sid).unwrap();
    assert_eq!(h[0].role, "system");
    assert!(h[0].content.contains("你负责a"));
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    // 回归：用户发言必须入转录（此前只进历史、不进转录，历史回放会丢用户消息）。
    match &events[0] {
        SessionEvent::Transcript(lines) => assert_eq!(lines[0].line, "[用户] 在吗"),
        _ => panic!("首条应为用户转录行"),
    }
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("[a]")))));
}

#[test]
fn single_mode_accepts_multi_module_agent_and_converses() {
    // 单 agent 形态允许 1 个 agent 带多个模块：能建、system 并入全部职责、说话人是 agent 名。
    let mut member = BTreeMap::new();
    member.insert("组合".to_string(), vec!["{\"type\":\"say\",\"text\":\"收到\"}".to_string()]);
    let mut core = core_with(vec![module_of("a"), module_of("b")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a", "b"])).unwrap().sid;
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.mode, "single");
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].modules, vec!["a".to_string(), "b".to_string()]);
    let h = core.single_history(&sid).unwrap();
    assert_eq!(h[0].role, "system");
    assert!(h[0].content.contains("你负责a") && h[0].content.contains("你负责b"), "system 必须并入全部模块职责");
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.starts_with("[组合] ")))),
        "转录说话人必须是 agent 名：{:?}",
        events
    );
}

#[test]
fn mode_vocabulary_is_single_or_collab_only() {
    // web 与 core 的形态词汇只有 single / collab；旧的 direct / compose 一概不认（GREEN FIELD，无兼容）。
    use crate::presentation::web::parse_mode;
    assert!(matches!(parse_mode("single"), Ok(WorkMode::Single)));
    assert!(matches!(parse_mode("collab"), Ok(WorkMode::Collab)));
    for bad in ["direct", "compose", "omni", ""] {
        let e = parse_mode(bad).unwrap_err();
        assert!(e.contains("未知模式"), "web 必须 400 并说明：{}", e);
    }
    // 落盘 meta 里的旧形态不做兼容：重建会话时明确报错（不静默当单 agent）。
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    hist.create(&SessionMeta {
        name: "旧会话".to_string(),
        mode: "direct".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 1,
        agents: vec![AgentMeta { name: "a".to_string(), transient: true, modules: vec!["a".to_string()], model: None }],
        exec: ExecSpec::default(),
    })
    .unwrap();
    // 内存里没有这个会话 → 走 rebuild_session，对未知形态如实报错。
    let err = core.rewind("旧会话", 0).unwrap_err();
    assert!(err.contains("未知会话形态"), "{}", err);
}

#[test]
fn core_collab_demo_runs_full_five_stages() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意方案\"}".to_string(),
    ]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]".to_string(),
    ]));
    let sid = core.create_work(collab_work("w", &["a"], false, "做个东西")).unwrap().sid;
    assert!(matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))));
    let events = core.collab_continue(&sid, CollabStep::Begin, "yes").unwrap();
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Plan(_))));
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
    // 终结会话已被中心回收（再查询挂起状态应报无此会话）。
    assert!(core.collab_pending(&sid).is_err());
}

#[test]
fn core_collab_delegated_slate_flow() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
    ]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        // 代拟（组装一个 agent）→ 整理 → 验收。
        "{\"picks\":[{\"name\":\"a\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
    ]));
    let opened = core.create_work(collab_work("w", &[], true, "做个东西")).unwrap();
    let sid = opened.sid;
    let ev = opened.events;
    assert!(matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmSlate))));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("[代拟]")))));
    let _ = core.collab_continue(&sid, CollabStep::ConfirmSlate, "yes").unwrap();
    assert!(matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))));
    let events = core.collab_continue(&sid, CollabStep::Begin, "yes").unwrap();
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
}

#[test]
fn core_collab_slate_rejects_invalid_picks() {
    // 不存在的模块 / 不存在的模型 / 不在登记处的复用项：整条拒收，合法的留下。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"picks\":[{\"name\":\"鬼\",\"modules\":[\"ghost\"],\"model\":\"m\",\"why\":\"模块不存在\"},{\"agent\":\"幽灵\",\"why\":\"不在登记处\"},{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"nope\",\"why\":\"模型不存在\"},{\"name\":\"乙\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案\"}".to_string(),
        "[{\"item\":\"x\",\"status\":\"pass\"}]".to_string(),
    ]));
    let ev = core.create_work(collab_work("w", &[], true, "任务")).unwrap().events;
    for want in ["ghost 不存在", "幽灵 不在登记处", "nope 不存在"] {
        assert!(ev.iter().any(|e| matches!(e, SessionEvent::Notice(n) if n.contains(want))), "应如实拒收：{}", want);
    }
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("乙〈a〉→ m")))));
}

#[test]
fn collab_delegated_roster_written_back_and_rebuilt_from_meta() {
    // 发言席是 agent：脚本按 agent 名 "调研员" 回放（不再按模块 id）。
    let mut member = BTreeMap::new();
    member.insert("调研员".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
    ]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        "{\"picks\":[{\"name\":\"调研员\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
    ]));
    let sid = core.create_work(collab_work("w", &[], true, "做个东西")).unwrap().sid;
    assert!(
        core.history_open("w").unwrap().0.agents.is_empty(),
        "确认名单之前不落档（名单只活在内存里）"
    );
    // CLI 的确认门要能把这份表单逐行读出来。
    let slate = core.collab_slate(&sid).unwrap();
    assert_eq!(slate.len(), 1);
    assert_eq!(slate[0].name, "调研员");
    assert!(slate[0].transient, "组装项如实标记为临时 agent");
    assert_eq!(slate[0].model.as_deref(), Some("m"));

    core.collab_continue(&sid, CollabStep::ConfirmSlate, "yes").unwrap();
    // 确认后名单写回 meta（重启/回档后的权威来源）。
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研员");
    assert_eq!(meta.agents[0].modules, vec!["a".to_string()]);
    assert_eq!(meta.agents[0].model.as_deref(), Some("m"));
    assert_eq!(meta.modules, vec!["a".to_string()]);

    // 回档（保留需求行）= 按 meta.agents 重建会话（协作走「按转录重建」这条路），再跑到交付。
    core.rewind(&sid, 1).unwrap();
    assert!(matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))), "重建后仍等确认开始");
    let events = core.collab_continue(&sid, CollabStep::Begin, "yes").unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "重建出来的名单要能一路跑完：{:?}",
        events
    );
}

// ---------- 平衡提取器 ----------

#[test]
fn extract_balanced_array() {
    let s = "前缀 [ {\"a\":1}, {\"b\":\"}\"} ] 后缀";
    let got = crate::core::envelope::extract_json_array(s).unwrap();
    assert!(got.starts_with('[') && got.ends_with(']'));
    let obj = crate::core::envelope::extract_json_object("x {\"k\":\"{\"} y").unwrap();
    assert!(obj.starts_with('{') && obj.ends_with('}'));
}

// ---------- 工具执行器 ----------

/// 守护 runner：任何调用即失败（守护不该用工具的路径）。
pub(crate) struct SilentRunner;
impl ToolRunner for SilentRunner {
    fn run(&self, _fence: &crate::core::fence::FenceSpec, _command: &str, _args: &str) -> ToolOutcome {
        panic!("不应调用工具");
    }
}

/// 记录型 runner：记录 (root, command, args)，回放固定输出。
pub(crate) struct RecordingRunner {
    pub(crate) calls: Mutex<Vec<(PathBuf, String, String)>>,
    out: String,
    ok: bool,
}
impl RecordingRunner {
    pub(crate) fn new(out: &str, ok: bool) -> RecordingRunner {
        RecordingRunner { calls: Mutex::new(Vec::new()), out: out.to_string(), ok }
    }
}
impl ToolRunner for RecordingRunner {
    fn run(&self, fence: &crate::core::fence::FenceSpec, command: &str, args_json: &str) -> ToolOutcome {
        // 记下工具进程的工作目录（= 该模块的根）与命令、参数。
        self.calls
            .lock()
            .expect("锁")
            .push((fence.cwd.clone(), command.to_string(), args_json.to_string()));
        ToolOutcome { ok: self.ok, output: self.out.clone() }
    }
}

const TOOL_CALL: &str = "{\"type\":\"tool\",\"name\":\"grep\",\"args\":{\"keyword\":\"x\"}}";

/// 带工具环境的成员：模块 m0 声明 grep → python tools/grep.py（cwd = 该模块目录）。
fn member_with_tools(id: &str, script: Vec<String>, runner: Arc<impl ToolRunner + Send + Sync + 'static>) -> Member {
    let mut commands = BTreeMap::new();
    commands.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut modules = BTreeMap::new();
    modules.insert("m0".to_string(), ModuleTools { root: abs(&["mods", "root"]), commands });
    let mut m = Member::new(id, "职责".to_string(), scripted(script));
    // 该路径走模块声明的外部命令（grep）：空沙箱 + 内存 IO，内置工具不参与。
    m.tools = Some(MemberTools {
        modules,
        runner,
        sandbox: test_sandbox("m0", &[]),
        io: Arc::new(InMemorySysIo::new()),
        unavailable: BTreeMap::new(),
        fence: crate::core::fence::FenceSpec::from_sandbox(&test_sandbox("m0", &[]), false),
    });
    m
}

/// 两个模块可以声明**同名**工具（信封里的 module 消歧）；每次调用跑在各自模块的目录里。
#[test]
fn same_named_tools_in_two_modules_run_in_their_own_root() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut member = BTreeMap::new();
    member.insert("组合".to_string(), vec![
        "{\"type\":\"tool\",\"module\":\"a\",\"name\":\"read_txt\",\"args\":{}}".to_string(),
        "{\"type\":\"tool\",\"module\":\"b\",\"name\":\"read_txt\",\"args\":{}}".to_string(),
        "{\"type\":\"say\",\"text\":\"两份都读完了\"}".to_string(),
    ]);
    let mut a = module_of("a");
    a.root = abs(&["mods", "a"]);
    a.manifest.tools.insert("read_txt".to_string(), "python tools/read_txt.py".to_string());
    let mut b = module_of("b");
    b.root = abs(&["mods", "b"]);
    b.manifest.tools.insert("read_txt".to_string(), "python tools/read_txt.py".to_string());
    let mut core = core_with_runner(vec![a, b], gw(member, vec!["[]".into()]), Arc::clone(&runner));
    // 跨模块同名不再算冲突：照样能建工作。
    let sid = core.create_work(work("w", WorkMode::Single, &["a", "b"])).unwrap().sid;
    // 提示词里的工具清单按模块分组，模型照此写 module。
    let h = core.single_history(&sid).unwrap();
    assert!(h[0].content.contains("- a：read_txt") && h[0].content.contains("- b：read_txt"), "清单要按模块分组：{}", h[0].content);
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    // 信封写了 module → tool 行呈现成 模块.工具（用户一眼看出调的是谁的）。
    let tool_lines = tool_line_texts(&events);
    assert_eq!(tool_lines.len(), 2, "两次调用 = 两条 tool 行：{:?}", tool_lines);
    assert!(tool_lines[0].contains("a.read_txt"), "{:?}", tool_lines);
    assert!(tool_lines[1].contains("b.read_txt"), "{:?}", tool_lines);
    assert!(tool_lines.iter().all(|l| l.contains("成功")), "{:?}", tool_lines);
    let calls = runner.calls.lock().expect("锁").clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, "python tools/read_txt.py");
    assert_eq!(calls[0].0, abs(&["mods", "a"]), "module=a 的 read_txt 跑在 a 的目录");
    assert_eq!(calls[1].1, "python tools/read_txt.py");
    assert_eq!(calls[1].0, abs(&["mods", "b"]), "module=b 的同名工具跑在 b 的目录（不共用 a 的 cwd）");
}

/// 多模块 agent 下省略 module：核心不猜，如实报错并把可用的「模块.工具」列全。
#[test]
fn external_tool_without_module_is_refused_when_agent_has_many_modules() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut member = BTreeMap::new();
    member.insert("组合".to_string(), vec![
        "{\"type\":\"tool\",\"name\":\"read_txt\",\"args\":{}}".to_string(),
        "{\"type\":\"say\",\"text\":\"知道了\"}".to_string(),
    ]);
    let mut a = module_of("a");
    a.manifest.tools.insert("read_txt".to_string(), "python tools/read_txt.py".to_string());
    let mut b = module_of("b");
    b.manifest.tools.insert("read_txt".to_string(), "python tools/read_txt.py".to_string());
    let mut core = core_with_runner(vec![a, b], gw(member, vec!["[]".into()]), Arc::clone(&runner));
    let sid = core.create_work(work("w", WorkMode::Single, &["a", "b"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    assert!(runner.calls.lock().expect("锁").is_empty(), "不猜模块，绝不落进程");
    let h = core.single_history(&sid).unwrap();
    let feedback = h
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[工具结果] read_txt"))
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(feedback.contains("没有指明所属模块"), "{}", feedback);
    // 失败也要落一条 tool 行（用户看得到这次调用没成）。
    let lines = tool_line_texts(&events);
    assert!(lines.iter().any(|l| l.contains("read_txt") && l.contains("失败")), "失败的调用同样要发 tool 行：{:?}", lines);
    assert!(feedback.contains("a.read_txt") && feedback.contains("b.read_txt"), "要把可用的 模块.工具 列全：{}", feedback);
    assert!(feedback.contains("read") && feedback.contains("write"), "内置工具也要列出：{}", feedback);
}

#[test]
fn tool_call_event_is_emitted_before_the_next_round() {
    // 短暂事件：工具跑完立刻发 tool_call（不落盘），供活动会话实时刷新。
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![TOOL_CALL.into(), "{\"type\":\"say\",\"text\":\"完成\"}".into()]);
    let mut mod_a = module_of("a");
    mod_a.manifest.tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), runner);
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let mut seen: Vec<&str> = Vec::new();
    {
        let mut emit = |e: SessionEvent| {
            seen.push(match e {
                SessionEvent::ToolCall(_) => "tool_call",
                SessionEvent::Transcript(_) => "transcript",
                SessionEvent::Notice(_) => "notice",
                SessionEvent::Delta { .. } => "delta",
                _ => "other",
            });
        };
        let mut live = Live { stream: false, cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)), emit: &mut emit };
        core.single_say(&sid, "跑一下", &mut live).unwrap();
    }
    assert!(seen.contains(&"tool_call"), "工具调用要发短暂 tool_call 事件：{:?}", seen);
}

#[test]
fn refs_rewrite_covers_prefixes_speakers_and_punctuation() {
    use crate::core::refs::{rewrite, RefRoots};
    // 文案来自提示词册：期望值也用册子渲染出来，代码里不复制那两句中文。
    let t = test_prompts().core.refs;
    let foreign = |path: &str, agent: &str| {
        crate::core::prompt::render(&t.foreign_sandbox, &[("agent", agent.to_string()), ("path", path.to_string())])
            .expect("册子变量齐全")
    };
    let collab = |path: &str, agent: &str| {
        crate::core::prompt::render(&t.collab_sandbox, &[("path", path.to_string()), ("agent", agent.to_string())])
            .expect("册子变量齐全")
    };
    assert!(
        t.foreign_sandbox.contains("{{agent}}") && t.foreign_sandbox.contains("{{path}}"),
        "无权文案要同时报出文件与 agent"
    );
    // 真实根：共享区 + 本 agent（甲）的私有沙箱
    let roots = RefRoots { work: abs(&["w", "work"]), private: Some(abs(&["w", "甲"])) };
    let collab_roots = RefRoots { work: abs(&["w", "work"]), private: None };
    // @work：与 speaker 无关，一律给出共享区真实路径
    assert_eq!(rewrite("@work:a.txt", Some("甲"), &roots, &t), s(&["w", "work", "a.txt"]));
    assert_eq!(rewrite("@work:sub/a.txt", None, &roots, &t), s(&["w", "work", "sub", "a.txt"]));
    // @sandbox：命中自己的沙箱 → 私有沙箱真实路径
    assert_eq!(rewrite("@sandbox:甲/note.txt", Some("甲"), &roots, &t), s(&["w", "甲", "note.txt"]));
    // @sandbox：别人的沙箱 → 册子文案（带文件名与 agent 名，不泄漏真实路径）
    assert_eq!(rewrite("@sandbox:乙/note", Some("甲"), &roots, &t), foreign("note", "乙"));
    // @sandbox：协作（没有"自己的沙箱"）→ 册子里的协作文案
    assert_eq!(rewrite("@sandbox:乙/note", None, &collab_roots, &t), collab("note", "乙"));
    // 终止标点留在原文里（只替换前缀+路径），所以句中标点/收尾标点都原样保留
    assert_eq!(rewrite("@work:a.txt，", Some("甲"), &roots, &t), format!("{}，", s(&["w", "work", "a.txt"])));
    assert_eq!(rewrite("看 @sandbox:甲/b.txt。", Some("甲"), &roots, &t), format!("看 {}。", s(&["w", "甲", "b.txt"])));
    assert_eq!(rewrite("@work:a.txt 请读它", Some("甲"), &roots, &t), format!("{} 请读它", s(&["w", "work", "a.txt"])));
    assert_eq!(rewrite("@work:a.txt，请读它", Some("甲"), &roots, &t), format!("{}，请读它", s(&["w", "work", "a.txt"])));
    // 句点不是终止符：note.txt 是完整路径
    assert_eq!(rewrite("@work:note.txt", Some("甲"), &roots, &t), s(&["w", "work", "note.txt"]));
    assert_eq!(
        rewrite("@sandbox:乙/note.txt", Some("甲"), &roots, &t),
        foreign("note.txt", "乙"),
        "扩展名要算进路径，不能截成 note"
    );
    assert_eq!(rewrite("@sandbox:乙/\"note.txt\"", Some("甲"), &roots, &t), foreign("note.txt", "乙"));
    // 已知取舍：句尾英文句点算进路径（要精确表达就用引号形式）
    assert_eq!(rewrite("@work:a.txt.", Some("甲"), &roots, &t), s(&["w", "work", "a.txt."]));
    assert_eq!(rewrite("@work:\"a.txt.\"", Some("甲"), &roots, &t), s(&["w", "work", "a.txt."]));
    // 引号形式：空白与标点都算路径的一部分
    assert_eq!(rewrite("@work:\"项目 说明.md\"", Some("甲"), &roots, &t), s(&["w", "work", "项目 说明.md"]));
    assert_eq!(rewrite("@work:\"a,b(1).md\"", Some("甲"), &roots, &t), s(&["w", "work", "a,b(1).md"]));
    assert_eq!(rewrite("@sandbox:甲/\"a b.md\"", Some("甲"), &roots, &t), s(&["w", "甲", "a b.md"]));
    assert_eq!(rewrite("@work:\"a b.md\" 看一下", Some("甲"), &roots, &t), format!("{} 看一下", s(&["w", "work", "a b.md"])));
    // agent 名也支持引号（名字含空白）：引号后必须紧跟 /
    assert_eq!(rewrite("@sandbox:\"调研 助手\"/\"a b.md\"", Some("调研 助手"), &roots, &t), s(&["w", "甲", "a b.md"]));
    assert_eq!(rewrite("@sandbox:\"调研 助手\"/a", Some("甲"), &roots, &t), foreign("a", "调研 助手"));
    // 根还没就绪（代拟确认前）：原样保留引用，不编路径
    assert_eq!(rewrite("@work:a.txt", None, &RefRoots::default(), &t), "@work:a.txt");
    // 不完整前缀 / 结构不满足 / 引号未闭合：原样输出（不猜）
    assert_eq!(rewrite("@work:", Some("甲"), &roots, &t), "@work:");
    assert_eq!(rewrite("@work: ", Some("甲"), &roots, &t), "@work: ");
    assert_eq!(rewrite("@sandbox:甲", Some("甲"), &roots, &t), "@sandbox:甲", "缺相对路径");
    assert_eq!(rewrite("@sandbox:/a.txt", Some("甲"), &roots, &t), "@sandbox:/a.txt", "缺 agent 名");
    assert_eq!(rewrite("@sandbox:甲/", Some("甲"), &roots, &t), "@sandbox:甲/", "相对路径为空");
    assert_eq!(rewrite("@work:\"a b.md", Some("甲"), &roots, &t), "@work:\"a b.md", "路径引号未闭合");
    assert_eq!(rewrite("@work:\"\"", Some("甲"), &roots, &t), "@work:\"\"", "空引号路径");
    assert_eq!(rewrite("@sandbox:\"调研 助手/a.md", Some("甲"), &roots, &t), "@sandbox:\"调研 助手/a.md", "agent 名引号未闭合");
    assert_eq!(rewrite("@sandbox:\"调研\"x/a.md", Some("甲"), &roots, &t), "@sandbox:\"调研\"x/a.md", "agent 名引号后缺 /");
    // 普通文本 / 误伤：不含两种前缀一律原样
    assert_eq!(rewrite("", Some("甲"), &roots, &t), "");
    assert_eq!(rewrite("没有引用", Some("甲"), &roots, &t), "没有引用");
    assert_eq!(rewrite("email@xxx.com 是我的", Some("甲"), &roots, &t), "email@xxx.com 是我的");
    assert_eq!(rewrite("@workx:a.txt", Some("甲"), &roots, &t), "@workx:a.txt");
    // 一句里多个引用；引号形式与无引号旧形式并存
    assert_eq!(
        rewrite("@work:a.txt 和 @sandbox:甲/b.txt", Some("甲"), &roots, &t),
        format!("{} 和 {}", s(&["w", "work", "a.txt"]), s(&["w", "甲", "b.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt 与 @work:\"a b.md\"", Some("甲"), &roots, &t),
        format!("{} 与 {}", s(&["w", "work", "a.txt"]), s(&["w", "work", "a b.md"]))
    );
}

#[test]
fn user_at_reference_is_rewritten_in_transcript_and_history() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["{\"type\":\"say\",\"text\":\"好的\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "@work:a.txt 看一下", l)).unwrap();
    // 转录的用户行已是确切寻址（不再是 @ 引用）
    let rows = transcript_rows(&events);
    let want = format!("[用户] {} 看一下", s(&["w", "work", "a.txt"]));
    assert_eq!(rows[0].1, want, "{:?}", rows);
    // 进上下文的是同一份文本（转录即内容）
    let h = core.single_history(&sid).unwrap();
    let want_msg = format!("{} 看一下", s(&["w", "work", "a.txt"]));
    assert!(h.iter().any(|m| m.role == "user" && m.content == want_msg), "{:?}", h);
    assert!(!h.iter().any(|m| m.content.contains("@work:")), "历史里不得残留 @ 引用：{:?}", h);
}

#[test]
fn workspace_list_reports_work_and_agent_files() {
    let ws = InMemoryWorkspace::new();
    ws.seed("w", "work", "投喂.txt");
    ws.seed("w", "work", "sub/b.txt");
    ws.seed("w", "甲", "note.txt");
    ws.seed("w", "乙", "x.txt");
    let f = ws.list("w", &["甲".to_string(), "乙".to_string(), "丙".to_string()]).unwrap();
    assert_eq!(f.work, vec!["sub/b.txt".to_string(), "投喂.txt".to_string()], "work 清单排序稳定");
    assert_eq!(f.agents["甲"], vec!["note.txt".to_string()]);
    assert_eq!(f.agents["乙"], vec!["x.txt".to_string()]);
    assert!(f.agents["丙"].is_empty(), "没有文件的 agent 也要在清单里（空表）");
}

#[test]
fn core_files_view_carries_lists_and_real_roots() {
    let ws = Arc::new(InMemoryWorkspace::new());
    let mut core = core_with_workspace(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]), Arc::clone(&ws));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    ws.seed("w", "work", "投喂.txt");
    ws.seed("w", "a", "note.txt");
    ws.seed("w", "b", "别人的.txt"); // 不属于本工作的 agent → 不该出现
    let f = core.files_view(&sid).unwrap();
    assert_eq!(f.work, vec!["投喂.txt".to_string()]);
    assert_eq!(f.agents.len(), 1, "只列本工作的 agent：{:?}", f.agents);
    assert_eq!(f.agents[0].name, "a");
    assert_eq!(f.agents[0].files, vec!["note.txt".to_string()]);
    // roots：真实绝对路径、/ 书写形式、与 agents 同序同名
    assert_eq!(f.roots.work, s(&["w", "work"]), "共享区根 = Sandboxes.shared");
    assert_eq!(f.roots.agents.len(), f.agents.len());
    assert_eq!(
        f.roots.agents.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        f.agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        "roots.agents 与 agents 必须同序同名"
    );
    assert_eq!(f.roots.agents[0].root, s(&["w", "a"]), "agent 根 = 它的私有沙箱");
    for root in std::iter::once(&f.roots.work).chain(f.roots.agents.iter().map(|r| &r.root)) {
        assert!(std::path::Path::new(root).is_absolute(), "根必须是绝对路径：{}", root);
        assert!(!root.contains('\\'), "根一律 / 分隔：{}", root);
        assert!(!root.starts_with("\\\\?\\"), "不得带扩展长度前缀：{}", root);
    }
    assert!(core.files_view("不存在的工作").is_err(), "无此会话要如实报错");
}

/// 真实的坏信封：结尾多了一个 ]（括号不配对）。路径用真实绝对路径（模型写对了路径、写坏了信封）。
/// 坏的形状照真案例：args 先闭合、多一个 ]、外层才闭合（整段不是合法 JSON）。
fn broken_tool(path: &str) -> String {
    format!("{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}]}}", path)
}

#[test]
fn malformed_tool_envelope_becomes_a_failed_tool_line() {
    // 真实案例：模型想调 write，但信封 JSON 非法（结尾多个 ]）——
    // 必须记一条 ok=false 的 tool 行，绝不把 JSON 当 AI 消息渲染，也绝不执行工具。
    let catalogue = test_prompts().core.tool_texts.malformed_note;
    let raw_path = s(&["w", "work", "README.md"]);
    let broken = broken_tool(&raw_path);
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![broken.clone(), "{\"type\":\"say\",\"text\":\"改好了\"}".to_string()]);
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "写个 README", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(rows.iter().filter(|r| r.2).count(), 1, "应有一条 tool 行：{:?}", rows);
    assert!(!rows.iter().any(|r| r.1.contains("\"type\"")), "JSON 绝不能当文本渲染：{:?}", rows);
    let views = tool_views(&events);
    assert_eq!(views.len(), 1);
    assert!(!views[0].ok, "非法的调用必须记为失败");
    assert_eq!(views[0].name, "write", "名字要尽力打捞出来（只用于显示）");
    assert_eq!(views[0].module, "", "没写 module → 空串");
    assert_eq!(views[0].output, catalogue, "回注册子文案");
    assert_eq!(views[0].raw, broken, "原文留档（重建上下文用）");
    assert_eq!(io.get(&["w", "work", "README.md"]), None, "非法信封绝不执行工具");
    // 历史：assistant(原文) + [工具结果]（含册子文案）→ 模型下一轮能自己改
    let h = core.single_history(&sid).unwrap();
    assert!(h.iter().any(|m| m.role == "assistant" && m.content == broken));
    assert!(h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] write") && m.content.contains(&catalogue)), "{:?}", h);
    // 重建一致（重启后从落盘流水重建上下文）
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2.rewind(&sid, 4).unwrap();
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| h.iter().map(|m| (m.role.clone(), m.content.clone())).collect::<Vec<_>>();
    assert_eq!(key(&rebuilt), key(&h), "重建上下文必须与实时历史逐条一致");
}

#[test]
fn malformed_tool_without_salvageable_name_still_records_a_line() {
    // 断在半截、括号不平衡：打捞不到名字也不能 panic，仍是一条 ok=false 的 tool 行。
    let half = "{\"type\":\"tool\",\"args\":{";
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![half.to_string(), "{\"type\":\"say\",\"text\":\"知道了\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "跑一下", l)).unwrap();
    let views = tool_views(&events);
    assert_eq!(views.len(), 1, "仍要记一条：{:?}", transcript_rows(&events));
    assert!(!views[0].ok && views[0].name.is_empty(), "打捞不到名字就留空：{:?}", views[0]);
    assert!(!transcript_rows(&events).iter().any(|r| r.1.contains("\"type\"")), "JSON 不进文本");
}

#[test]
fn repeated_malformed_envelopes_hit_the_tool_cap_and_stop() {
    // 模型反复输出非法信封：计入工具上限，最终强制收尾，不会死循环。
    let broken = broken_tool(&s(&["w", "work", "README.md"]));
    let script: Vec<String> = (0..MAX_TOOL_CALLS + 1).map(|_| broken.clone()).collect();
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "一直错", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(rows.iter().filter(|r| r.2).count(), MAX_TOOL_CALLS, "上限内每次非法信封各记一条：{:?}", rows);
    assert!(!rows.iter().any(|r| r.1.contains("\"type\"")), "JSON 不进文本：{:?}", rows);
}

#[test]
fn tool_type_mention_in_plain_speech_is_not_misjudged() {
    // ① 合法 say 信封的正文里引用 {"type":"tool"}：正常发言，不是工具调用。
    let say = serde_json::json!({"type": "say", "text": "调用格式是 {\"type\":\"tool\"} 这样"}).to_string();
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![say]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "怎么写", l)).unwrap();
    assert!(tool_views(&events).is_empty(), "不该有工具行：{:?}", transcript_rows(&events));
    assert_eq!(transcript_rows(&events).iter().filter(|r| !r.2).count(), 2, "用户行 + 一条发言");

    // ② 普通正文（不以 { 开头）里提到它：仍按发言收录（降级），不误判。
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["写法示例：{\"type\":\"tool\"} 就是这样".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "再讲一次", l)).unwrap();
    assert!(tool_views(&events).is_empty(), "不以 JSON 为主体 → 不误判：{:?}", transcript_rows(&events));
    assert!(transcript_rows(&events).iter().any(|r| r.1.contains("写法示例")), "{:?}", transcript_rows(&events));
    // ③ 正文里引用一个**平衡**的完整对象：同样不误判（未闭合才判坏信封）。
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["格式是 {\"type\":\"tool\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "再示范一次", l)).unwrap();
    assert!(tool_views(&events).is_empty(), "平衡对象不算坏信封：{:?}", transcript_rows(&events));
    assert!(transcript_rows(&events).iter().any(|r| r.1.contains("格式是")), "{:?}", transcript_rows(&events));
}

#[test]
fn prose_then_unclosed_tool_envelope_is_malformed_and_keeps_prose() {
    // 正文在前、坏信封在后（未闭合）：也要判 malformed，且正文照常显示、JSON 不上屏。
    let raw = "好的。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\"}";
    let catalogue = test_prompts().core.tool_texts.malformed_note;
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![raw.to_string(), "{\"type\":\"say\",\"text\":\"改好了\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "写一下", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(rows.iter().filter(|r| r.2).count(), 1, "应有一条 tool 行：{:?}", rows);
    assert!(!rows.iter().any(|r| r.1.contains("\"type\"")), "JSON 绝不能上屏：{:?}", rows);
    assert!(rows.iter().any(|r| r.1 == "[a] 好的。"), "坏信封之前的正文要照常显示：{:?}", rows);
    let views = tool_views(&events);
    assert!(!views[0].ok && views[0].name == "write", "{:?}", views[0]);
    assert!(views[0].args.contains("\"path\":\"a\""), "参数尽力打捞：{:?}", views[0].args);
    assert_eq!(views[0].output, catalogue);
    // 历史：assistant(原文) + [工具结果]（模型下一轮能自己改）
    let h = core.single_history(&sid).unwrap();
    assert!(h.iter().any(|m| m.role == "assistant" && m.content == raw));
    assert!(h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] write")), "{:?}", h);
}

#[test]
fn envelope_tool_parses_name_and_args() {
    let r = crate::core::envelope::parse(TOOL_CALL);
    assert_eq!(r.verb, crate::core::envelope::Verb::Tool);
    let inv = r.tool.expect("应有调用申请");
    assert_eq!(inv.name, "grep");
    assert!(inv.module.is_none(), "没写 module = None（单模块 agent 靠这个兜底）");
    assert!(inv.args_json.contains("keyword"));
    // 带 module 的信封：trim 后非空才是 Some（空串按省略处理）。
    let with_mod = crate::core::envelope::parse("{\"type\":\"tool\",\"module\":\" reviewer \",\"name\":\"read_txt\",\"args\":{}}");
    assert_eq!(with_mod.tool.expect("应有调用申请").module.as_deref(), Some("reviewer"));
    let blank_mod = crate::core::envelope::parse("{\"type\":\"tool\",\"module\":\"  \",\"name\":\"read_txt\",\"args\":{}}");
    assert!(blank_mod.tool.expect("应有调用申请").module.is_none());
    // name 缺失 = 工具信封但不合法 → **独立的 malformed 信号**（不再按原文发言收录）。
    let bad = crate::core::envelope::parse("{\"type\":\"tool\",\"args\":{}}");
    assert_eq!(bad.verb, crate::core::envelope::Verb::Tool, "看得出是想发工具信封");
    assert!(!bad.degraded, "malformed 与 degraded 是两回事（后者是信封缺失）");
    let inv = bad.tool.expect("应给出非法信封信号");
    assert!(inv.malformed && inv.name.is_empty(), "打捞不到名字就留空：{:?}", inv);
    assert!(bad.text.is_empty(), "非法信封的 JSON 也不进 text");
    // 真的"没有信封"仍然是 degraded say（原文收录）。
    let plain = crate::core::envelope::parse("没有信封的发言");
    assert!(plain.degraded && plain.tool.is_none() && plain.text == "没有信封的发言");
    // 信封之外的正文才进 text（永不把信封 JSON 当文本）；只剩信封时 text 为空串。
    assert!(crate::core::envelope::parse(TOOL_CALL).text.is_empty(), "只剩信封 → text 空");
    let prose = crate::core::envelope::parse("先看一眼。{\"type\":\"tool\",\"name\":\"grep\",\"args\":{}}后记");
    assert_eq!(prose.text, "先看一眼。后记", "信封之外的正文进 text");
    assert!(!prose.text.contains('{') && !prose.text.contains("type"), "正文里不得残留 JSON：{}", prose.text);
}

#[test]
fn streaming_stops_at_the_envelope_brace() {
    // 正文开头照常流；一旦出现 "{"（信封开始）就不再外送后续片段。
    use crate::core::session::stream_piece;
    let (send, acc) = stream_piece("", "我先看看。");
    assert_eq!(send, "我先看看。");
    let (send, acc) = stream_piece(&acc, "{\"type\":\"tool\"}");
    assert!(send.is_empty(), "信封不外泄");
    let (send, _) = stream_piece(&acc, "后记");
    assert!(send.is_empty(), "出现过花括号之后一律不外送");
    // { 出现在片段中间：它之前的部分仍可外送
    let (send, acc) = stream_piece("", "正文{后面是信封}");
    assert_eq!(send, "正文");
    let (send, _) = stream_piece(&acc, "尾巴");
    assert!(send.is_empty());
    // 没有花括号的正文一路外送
    let (send, acc) = stream_piece("你好", "，世界");
    assert_eq!(send, "，世界");
    assert_eq!(acc, "你好，世界");
}

#[test]
fn envelope_only_round_produces_only_a_tool_line() {
    // 只有信封、没有正文也没有思维链 → 只出 tool 行，不产生空行。
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![TOOL_CALL.into(), "{\"type\":\"say\",\"text\":\"完成\"}".into()]);
    let mut mod_a = module_of("a");
    mod_a.manifest.tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), runner);
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let rows = transcript_rows(&with_live(|l| core.single_say(&sid, "跑一下", l)).unwrap());
    assert_eq!(rows.len(), 3, "用户行 / tool 行 / 答复行：{:?}", rows);
    assert_eq!(rows.iter().filter(|r| r.2).count(), 1, "只出一条 tool 行：{:?}", rows);
    assert!(rows[1].2, "tool 行居中：{:?}", rows);
}

#[test]
fn prose_then_tool_envelope_keeps_prose_line_and_rebuilds_identically() {
    // 同一轮里「先写正文再发信封」：正文与思维链要落进文本行，且正文行不得含 JSON；
    // 重建上下文必须与实时历史逐条一致（工具轮的文本行不另推 assistant）。
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let n = s(&["w", "a", "n.txt"]);
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        format!("我先看看这个文件。{{\"type\":\"tool\",\"module\":\"a\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}}}", n),
        "{\"type\":\"say\",\"text\":\"写好了\"}".to_string(),
    ]);
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(rows.len(), 4, "用户行 / 正文行 / tool 行 / 答复行：{:?}", rows);
    assert!(rows[1].1.contains("我先看看这个文件。"), "正文要落进文本行：{:?}", rows[1]);
    assert!(!rows[1].1.contains('{'), "正文行不得含 JSON：{:?}", rows[1]);
    assert!(!rows[1].2 && rows[2].2 && !rows[3].2, "tool 行紧随正文行：{:?}", rows);
    assert!(!rows[2].1.contains('{'), "tool 行也不含 JSON：{:?}", rows[2]);
    assert_eq!(io.get(&["w", "a", "n.txt"]).as_deref(), Some("hi"), "工具真的跑了");

    let live_h = core.single_history(&sid).unwrap();
    assert!(
        live_h.iter().any(|m| m.role == "assistant" && m.content.contains("我先看看这个文件。") && m.content.contains("\"type\":\"tool\"")),
        "这一轮的 assistant 消息 = assistant(raw)，正文与信封都在：{:?}",
        live_h
    );
    assert!(live_h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] write")), "{:?}", live_h);

    // 「重启」：同一份落盘历史交给新的 Core，重建后必须与实时历史逐条一致。
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2.rewind(&sid, 4).unwrap(); // 保留全部 4 行
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| h.iter().map(|m| (m.role.clone(), m.content.clone())).collect::<Vec<_>>();
    assert_eq!(key(&rebuilt), key(&live_h), "重建上下文必须与实时历史逐条一致");
}

#[test]
fn forced_final_tool_envelope_shows_no_json() {
    // 超限强制收尾后模型仍发信封：显示文本取信封之外的正文（没有正文就是空），JSON 绝不进转录。
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut script: Vec<String> = (0..MAX_TOOL_CALLS).map(|_| TOOL_CALL.to_string()).collect();
    script.push("到此为止。{\"type\":\"tool\",\"name\":\"grep\",\"args\":{}}".to_string());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut mod_a = module_of("a");
    mod_a.manifest.tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), Arc::clone(&runner));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "跑满", l)).unwrap();
    assert_eq!(runner.calls.lock().expect("锁").len(), MAX_TOOL_CALLS, "超限后不再执行工具");
    let rows = transcript_rows(&events);
    assert_eq!(rows.iter().filter(|r| r.2).count(), MAX_TOOL_CALLS, "{:?}", rows);
    let last = rows.last().expect("应有末行");
    assert!(last.1.contains("到此为止。"), "超限后的正文要如实收录：{:?}", last);
    assert!(!last.1.contains('{') && !last.1.contains("\"type\""), "显示文本不得出现 JSON：{:?}", last);
    assert!(!rows.iter().any(|r| r.1.contains("\"type\"")), "整条转录都不得出现 JSON：{:?}", rows);
}

#[test]
fn tool_loop_runs_declared_tool() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "  3 | 命中行".into(), ok: true });
    let mut m = member_with_tools(
        "m0",
        vec![TOOL_CALL.into(), "{\"type\":\"say\",\"text\":\"完成\"}".into()],
        Arc::clone(&runner),
    );
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("完成"));
    let calls = runner.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1, "声明过的工具应恰好执行一次");
    assert_eq!(calls[0].0, abs(&["mods", "root"]), "工具进程工作目录 = 模块工作区（真实路径）");
    assert_eq!(calls[0].1, "python tools/grep.py", "命令来自模块清单");
    assert!(calls[0].2.contains("keyword"), "参数以 JSON 原样送达");
    let trace = exec.traces.get("m0").expect("工具调用应入册");
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0].name, "grep");
    assert_eq!(trace[0].module, "m0", "信封省略 module 时按唯一模块兜底");
    assert!(trace[0].ok && trace[0].args.contains("keyword"), "{:?}", trace[0]);
    assert!(trace[0].raw.contains("\"type\":\"tool\""), "原始输出要留档（重建上下文用）");
}

#[test]
fn tool_loop_rejects_undeclared_tool() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: String::new(), ok: true });
    let mut m = member_with_tools(
        "m0",
        vec!["{\"type\":\"tool\",\"name\":\"nope\",\"args\":{}}".into(), "{\"type\":\"say\",\"text\":\"完成\"}".into()],
        Arc::clone(&runner),
    );
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(std::slice::from_mut(&mut m), "任务", &prompts);
    assert!(runner.calls.lock().expect("锁").is_empty(), "未声明的工具绝不落进程");
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("完成"));
    let trace = exec.traces.get("m0").expect("工具调用应入册");
    assert!(trace.iter().any(|v| v.output.contains("未声明")), "{:?}", trace.iter().map(|v| &v.output).collect::<Vec<_>>());
}

#[test]
fn shipped_modules_scan_clean() {
    // 随仓模块（modules/）是产品内容的一部分：清单必须全部合法、id 与目录一致。
    let roster = crate::adapters::FsModules::new(PathBuf::from("modules")).scan();
    assert!(!roster.modules.is_empty(), "仓库应自带模块");
    assert!(roster.rejected.is_empty(), "随仓清单必须全部合法：{:?}", roster.rejected);
    assert!(roster.modules.iter().any(|m| m.manifest.id == "summarizer"));
    assert!(roster.modules.iter().any(|m| m.manifest.id == "reviewer"));
}

#[test]
fn tool_loop_cap_forces_final_answer() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "r".into(), ok: true });
    let mut script: Vec<String> = (0..MAX_TOOL_CALLS).map(|_| TOOL_CALL.to_string()).collect();
    script.push("{\"type\":\"say\",\"text\":\"最终回报\"}".into());
    let mut m = member_with_tools("m0", script, Arc::clone(&runner));
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(runner.calls.lock().expect("锁").len(), MAX_TOOL_CALLS, "调用数封顶");
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("最终回报"), "超限后强制收尾");
}

#[test]
fn core_direct_tool_flow_injects_result_into_history() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "  3 | 依据行".into(), ok: true });
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        TOOL_CALL.to_string(),
        "{\"type\":\"say\",\"text\":\"依据第 3 行，结论成立\"}".to_string(),
    ]);
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut mod_a = module_of("a");
    mod_a.manifest.tools = manifest_tools;
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), Arc::clone(&runner));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "核对一下", l)).unwrap();
    // 一行 = 一轮模型调用：用户行 / tool 行 / 最终文本行，id 连续且 tool 行夹在中间。
    let rows = transcript_rows(&events);
    assert_eq!(rows.len(), 3, "一次工具循环应得到 3 条行：{:?}", rows);
    assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![0, 1, 2], "id 连续");
    assert!(!rows[0].2 && rows[1].2 && !rows[2].2, "tool 行夹在中间：{:?}", rows);
    assert!(rows[1].1.contains("grep") && rows[1].1.contains("成功"), "{:?}", rows[1]);
    assert!(rows[2].1.contains("结论成立"), "{:?}", rows[2]);
    // 历史完整：用户消息 → 工具信封原文 → 工具结果 → 最终答复。
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] a.grep")),
        "工具结果必须回注上下文（标签带模块）：{:?}",
        h
    );
    assert!(h.iter().any(|m| m.role == "assistant" && m.content.contains("结论成立")));
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}

// ---------- 内置文件工具：沙箱寻址与越界 ----------

#[test]
fn sandbox_resolve_accepts_only_absolute_paths_inside_roots() {
    use crate::core::workspace::Place;
    let sb = test_sandbox("a1", &["data"]);
    // 绝对且在根内 → 通过（返回归一化后的绝对路径）
    let (place, path) = sb.resolve(&p(&["demo", "work", "notes", "a.txt"])).expect("共享区可达");
    assert_eq!(place, Place::Shared);
    assert_eq!(path, abs(&["demo", "work", "notes", "a.txt"]));
    let (place, path) = sb.resolve(&p(&["demo", "a1", "b.txt"])).expect("私有沙箱可达");
    assert_eq!(place, Place::Private);
    assert_eq!(path, abs(&["demo", "a1", "b.txt"]));
    let (place, path) = sb.resolve(&p(&["mods", "data", "c.txt"])).expect("成员模块目录可达");
    assert_eq!(place, Place::Module("data".to_string()));
    assert_eq!(path, abs(&["mods", "data", "c.txt"]));
    // 允许的根就是这三处：错误文案要把它们列全
    let roots_line = s(&["demo", "work"]);
    // 拒绝：越界 / 非绝对路径（相对、裸文件名、带冒号前缀的伪路径）/ .. 与 . 段 / 空段 / 空
    let bad: Vec<String> = vec![
        p(&["outside", "x.txt"]),
        "a.txt".to_string(),
        "demo/work/a.txt".to_string(),
        "work:/a.txt".to_string(),
        "sandbox:/b.txt".to_string(),
        "module:data:/c.txt".to_string(),
        format!("{}{}..{}x.txt", p(&["demo", "work"]), std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR),
        format!("{}{}a{}..{}b.txt", p(&["demo", "work"]), std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR),
        format!("{}//a.txt", p(&["demo", "work"])),
        "   ".to_string(),
    ];
    for b in &bad {
        let err = sb.resolve(b).unwrap_err();
        assert!(err.contains(&roots_line), "错误要把允许的真实根列全（{}）：{}", b, err);
    }
}

#[test]
fn builtin_search_reports_line_numbers_and_respects_case() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(&["demo", "work", "note.txt"], "第一行 Alpha\n第二行 beta\nalpha 小写\n");
    let path = s(&["demo", "work", "note.txt"]);
    // 默认区分大小写
    let out = crate::core::systool::execute(&sb, &io, "search", &format!("{{\"path\":\"{}\",\"keyword\":\"alpha\"}}", path));
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("3 | alpha 小写"), "{}", out.output);
    assert!(!out.output.contains("第一行 Alpha"), "默认区分大小写：{}", out.output);
    assert!(out.output.contains("命中 1 行 / 全文 3 行"), "{}", out.output);
    // ignore_case = true
    let out = crate::core::systool::execute(
        &sb,
        &io,
        "search",
        &format!("{{\"path\":\"{}\",\"keyword\":\"alpha\",\"ignore_case\":true}}", path),
    );
    assert!(out.output.contains("1 | 第一行 Alpha") && out.output.contains("3 | alpha 小写"), "{}", out.output);
    assert!(out.output.contains("命中 2 行 / 全文 3 行"), "{}", out.output);
    // 越界被拒
    let bad = crate::core::systool::execute(
        &sb,
        &io,
        "search",
        &format!("{{\"path\":\"{}\",\"keyword\":\"x\"}}", s(&["outside", "f.txt"])),
    );
    assert!(!bad.ok && bad.output.contains("不在允许的根目录内"), "{}", bad.output);
}

#[test]
fn agent_system_carries_the_real_roots() {
    // 提示词里给出的根必须是真实绝对路径（模型据此拼路径；外部工具也认它）。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let h = core.single_history(&sid).unwrap();
    let system = &h[0].content;
    assert!(system.contains(&s(&["w", "work"])), "system 要含共享区真实根：{}", system);
    assert!(system.contains(&s(&["w", "a"])), "system 要含私有沙箱真实根：{}", system);
    assert!(system.contains(&format!("：{}", s(&["a"]))), "system 要含模块目录真实根：{}", system);
    // 示例必须用真实根拼
    assert!(
        system.contains(&format!("\"path\":\"{}/note.txt\"", s(&["w", "work"]))),
        "示例要用真实根拼路径：{}",
        system
    );
    // 护栏：提示词里不得出现任何解析不了的路径写法（模型会照抄它）
    assert!(!system.contains("work:/") && !system.contains("sandbox:/"), "提示词只该给真实根：{}", system);
}

#[test]
fn sandboxes_lookup_is_by_agent() {
    let boxes = crate::core::workspace::Sandboxes { shared: abs(&["w", "work"]), list: vec![test_sandbox("甲", &["a"])] };
    assert!(boxes.for_agent("甲").is_some(), "按 agent 实例名取沙箱");
    assert!(boxes.for_agent("a").is_none(), "沙箱不再按模块 id 反查");
}

#[test]
fn builtin_file_tools_run_in_direct_session() {
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["w", "a", "note.txt"]);
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            format!("{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"你好\"}}}}", note),
            format!("{{\"type\":\"tool\",\"name\":\"read\",\"args\":{{\"path\":\"{}\"}}}}", note),
            "{\"type\":\"say\",\"text\":\"已读写完\"}".to_string(),
        ],
    );
    let mut core = core_with_io(vec![module_of("a")], gw(member, vec!["[]".into()]), Arc::clone(&io));
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    // 落点 = session/<工作名>/<agent 实例名>/（测试里的临时 agent 名取模块名）
    assert_eq!(io.get(&["w", "a", "note.txt"]).as_deref(), Some("你好"));
    let tool_lines = tool_line_texts(&events);
    assert_eq!(tool_lines.len(), 2, "write + read = 两条 tool 行：{:?}", tool_lines);
    assert!(tool_lines[0].contains("write") && tool_lines[1].contains("read"), "{:?}", tool_lines);
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] read") && m.content.contains("你好")),
        "读回的内容必须回注上下文"
    );
}

#[test]
fn builtin_write_into_module_dir_is_allowed_with_notice() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &["data"]);
    let keep = s(&["mods", "data", "keep.txt"]);
    let out = crate::core::systool::execute(&sb, &io, "write", &format!("{{\"path\":\"{}\",\"content\":\"状态\"}}", keep));
    assert!(out.ok, "{}", out.output);
    assert_eq!(io.get(&["mods", "data", "keep.txt"]).as_deref(), Some("状态"));
    assert!(out.output.contains("模块 data"), "写模块目录要如实提示：{}", out.output);
    assert!(out.output.contains(crate::core::systool::MODULE_WRITE_MARK), "要有可供轨迹识别的标记：{}", out.output);
    // 越界写入：拒绝且不落盘。
    let other = s(&["mods", "other", "x.txt"]);
    let bad = crate::core::systool::execute(&sb, &io, "write", &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", other));
    assert!(!bad.ok);
    assert_eq!(io.get(&["mods", "other", "x.txt"]), None);
}

#[test]
fn builtin_read_reports_errors_verbatim() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(&["demo", "work", "note.txt"], "内容");
    let note = s(&["demo", "work", "note.txt"]);
    let ok = crate::core::systool::execute(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", note));
    assert!(ok.ok && ok.output.contains("内容"), "{}", ok.output);
    assert!(ok.output.contains(&note), "回执要写明读的是哪个文件：{}", ok.output);
    let missing = crate::core::systool::execute(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", s(&["demo", "work", "nope.txt"])));
    assert!(!missing.ok);
    assert!(missing.output.contains("不存在"), "{}", missing.output);
    // 参数不合法 = 如实报错，不猜用户想干什么。
    assert!(!crate::core::systool::execute(&sb, &io, "read", "{").ok);
    assert!(!crate::core::systool::execute(&sb, &io, "read", "{}").ok);
    // 非绝对路径一律拒绝（相对路径、带冒号前缀的伪路径都落在这里）。
    let rel = crate::core::systool::execute(&sb, &io, "read", "{\"path\":\"nope.txt\"}");
    assert!(!rel.ok && rel.output.contains("需要绝对路径"), "{}", rel.output);
    let fake = crate::core::systool::execute(&sb, &io, "read", "{\"path\":\"work:/nope.txt\"}");
    assert!(!fake.ok && fake.output.contains("需要绝对路径"), "{}", fake.output);
}

#[test]
fn core_collab_tool_modules_run_in_execution() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "  1 | 内容".into(), ok: true });
    let mut member = BTreeMap::new();
    // 脚本排布：讨论开场 say → 讨论 step agree（收敛）→ 执行阶段 TOOL_CALL → 最终回报。
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"建议直接做\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        TOOL_CALL.to_string(),
        "{\"type\":\"say\",\"text\":\"执行完毕，见依据\"}".to_string(),
    ]);
    let mut mod_a = module_of("a");
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    mod_a.manifest.tools = manifest_tools;
    // 核心脚本：整理（方案）→ 验收（全过）。
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec![
            "{\"type\":\"say\",\"text\":\"方案：查证后回报\"}".into(),
            "[{\"item\":\"查证\",\"status\":\"pass\"}]".into(),
        ]),
        Arc::clone(&runner),
    );
    let opened = core.create_work(collab_work("w", &["a"], false, "任务")).unwrap();
    let sid = opened.sid;
    let mut events = opened.events;
    events.extend(core.collab_continue(&sid, CollabStep::Begin, "").unwrap());
    // 工具只在执行阶段跑：发一条 tool 转录行 + 恰好一次进程调用。
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Transcript(ls)
            if ls.iter().any(|l| l.tool.is_some() && l.line.contains("[a:tool]") && l.line.contains("成功")))),
        "执行阶段的工具调用应发 tool 转录行",
    );
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })), "验收应通过");
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}
// ---------- 运行包：契约、包库、诊断、执行计划 ----------

#[test]
fn package_manifest_check_rejects_illegal_forms() {
    let check = crate::core::packages::check_manifest;
    assert!(check(&pkg("python", "3.12.4")).is_ok(), "prefix 类默认 kind");
    assert!(check(&pkg_yaml("id: Python\nversion: 1\nprefix: opt/p")).is_err(), "id 只允许小写");
    assert!(check(&pkg_yaml("id: py\nversion: 1\nkind: magic\nprefix: opt/p")).is_err(), "kind 只认 prefix / system");
    assert!(check(&pkg_yaml("id: py\nversion: 1")).is_err(), "prefix 类必须给 prefix");
    assert!(check(&pkg_yaml("id: py\nversion: 1\nprefix: opt/../etc")).is_err(), "前缀不能含 ..");
    assert!(check(&pkg_yaml("id: py\nversion: 1\nprefix: opt\\\\rt")).is_err(), "前缀用 / 书写形式");
    assert!(check(&pkg_yaml("id: cc\nversion: 1\nkind: system")).is_err(), "system 类必须给 provides_paths");
    assert!(check(&pkg_yaml("id: cc\nversion: 1\nkind: system\nprovides_paths: [usr/include]")).is_ok());
    assert!(check(&pkg_yaml("id: a\nversion: 1\nprefix: opt/a\nrequires: [a]")).is_err(), "requires 不能依赖自己");
}

#[test]
fn module_runtimes_are_validated() {
    let mut m = module_of("a");
    m.manifest.runtimes = vec!["python".to_string(), "cc".to_string()];
    assert!(crate::core::module::check_runtimes(&m.manifest).is_ok());
    m.manifest.runtimes = vec!["Python".to_string()];
    assert!(crate::core::module::check_runtimes(&m.manifest).is_err(), "大写不合法");
    m.manifest.runtimes = vec!["python".to_string(), "python".to_string()];
    assert!(crate::core::module::check_runtimes(&m.manifest).unwrap_err().contains("重复"), "重复声明要拒收");
}

#[test]
fn module_tools_may_not_take_builtin_names() {
    let mut m = module_of("a");
    m.manifest.tools.insert("read_txt".to_string(), "python tools/read_txt.py".to_string());
    assert!(crate::core::module::check_tools(&m.manifest).is_ok(), "普通工具名可用");
    for name in ["read", "write", "search"] {
        m.manifest.tools.insert(name.to_string(), "python tools/x.py".to_string());
        let why = crate::core::module::check_tools(&m.manifest).unwrap_err();
        assert!(why.contains("保留名"), "内置工具名要拒收：{}", why);
        m.manifest.tools.remove(name);
    }
}

#[test]
fn library_keeps_versions_and_rejects_duplicates() {
    let lib = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9"), pkg("python", "3.12.4"), pkg_yaml("id: bad\nversion: 1")],
        vec!["x：package.yaml 非法".to_string()],
    );
    let versions: Vec<&str> = lib.versions_of("python").iter().map(|p| p.version.as_str()).collect();
    assert_eq!(versions, vec!["3.11.9", "3.12.4"], "同 (id, version) 只收一份，版本升序");
    assert!(lib.rejected.iter().any(|r| r.contains("只收先出现的那份")), "{:?}", lib.rejected);
    assert!(lib.rejected.iter().any(|r| r.contains("prefix")), "非法清单要说明原因：{:?}", lib.rejected);
    assert!(lib.rejected.iter().any(|r| r.contains("package.yaml 非法")), "适配层拒收原因也要留：{:?}", lib.rejected);
    let caps = lib.capability_versions();
    assert_eq!(caps.get("python").map(|v| v.len()), Some(2));
}

#[test]
fn package_conflicts_flag_overlapping_paths() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: a\nversion: 1\nkind: system\nprovides_paths: [usr/lib]"),
            pkg_yaml("id: b\nversion: 1\nkind: system\nprovides_paths: [usr/lib/x86_64]"),
            pkg("node", "20.11.1"),
        ],
        Vec::new(),
    );
    let refs: Vec<&PackageManifest> = lib.packages.iter().collect();
    let got = crate::core::packages::conflicts(&refs);
    assert_eq!(got.len(), 1, "只有那对写进同一处的包冲突：{:?}", got);
    assert_eq!(got[0].0, "usr/lib");
    assert!(got[0].1.contains("a@1") && got[0].2.contains("b@1"), "{:?}", got);
}

/// 虚拟机档选型：基础根 + 不联网（定版留空 = 让库自己决定；多版本时报歧义）。
fn vm_spec() -> ExecSpec {
    ExecSpec { tier: Tier::Vm, base: Some("base-linux".to_string()), pins: BTreeMap::new(), net: false }
}

#[test]
fn exec_host_tier_ignores_packages() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let plan = exec::plan(&ExecSpec::default(), &modules, &Library::default())
        .expect("本机档不装载运行包，不会因缺包失败");
    assert_eq!(plan.tier, Tier::Host);
    assert!(plan.packages.is_empty());
    assert!(plan.base.is_none());
    assert!(!plan.net, "默认不放行出站网络");
    let summary = exec::plan_summary(&plan);
    assert!(summary.contains("本机") && summary.contains("不放行"), "{}", summary);
}

#[test]
fn exec_vm_tier_reports_missing_ambiguous_and_unavailable() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let empty = Library::default();
    let spec = vm_spec();
    assert_eq!(
        exec::vm_diagnoses(&modules, &empty, &spec),
        vec![Diagnosis::Missing { module: "a".to_string(), capability: "python".to_string() }]
    );
    let un = exec::unavailable(&spec, &modules, &empty);
    assert_eq!(un.get("a"), Some(&vec!["python".to_string()]), "虚拟机档缺包 = 该模块工具不可用");
    assert!(exec::unavailable(&ExecSpec::default(), &modules, &empty).is_empty(), "本机档一律可用");
    // 缺包不拦会话：计划里没有可装载的包，那一步的降级由 unavailable 收口。
    let partial = exec::plan(&spec, &modules, &empty).expect("缺包不该拦会话");
    assert!(partial.packages.is_empty());
    let said = exec::diagnose_text(&exec::vm_diagnoses(&modules, &empty, &spec));
    assert!(said.contains("runtimes/"), "缺包的说法要告诉用户把包放哪：{}", said);
    // 多版本且未定版 = 不替用户选
    let two = Library::build(vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")], Vec::new());
    assert!(exec::vm_diagnoses(&modules, &two, &spec).iter().any(|d| matches!(d, Diagnosis::Ambiguous { .. })));
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec { pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]), ..vm_spec() };
    assert!(exec::vm_diagnoses(&modules, &two, &bad).iter().any(|d| matches!(d, Diagnosis::UnknownPin { .. })));
    // 多版本未定版 = 选型不成立：派生计划如实拒绝（这是用户要解决的选型问题，不是"缺包"）
    let refused = exec::plan(&spec, &modules, &two).unwrap_err();
    assert!(exec::diagnose_text(&refused).contains("多个版本"), "{}", exec::diagnose_text(&refused));
    // 定版之后可以成立
    let ok = ExecSpec { pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]), ..vm_spec() };
    assert!(exec::vm_diagnoses(&modules, &two, &ok).is_empty());
    assert_eq!(exec::plan(&ok, &modules, &two).unwrap().packages.len(), 1);
}

#[test]
fn exec_vm_plan_pins_versions_and_orders_prefix_before_system() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/include]\nrequires: [binutils]"),
            pkg_yaml("id: binutils\nversion: 2.42\nprefix: opt/rt/binutils2.42"),
            pkg("python", "3.12.4"),
        ],
        Vec::new(),
    );
    let modules = vec![module_with_runtimes("a", &["python", "cc"])];
    let plan = exec::plan(&vm_spec(), &modules, &lib).expect("虚拟机档可成立");
    let ids: Vec<&str> = plan.packages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["binutils", "python", "cc"], "先独立前缀，后写进系统路径的包；包的 requires 走闭包");
    assert_eq!(plan.packages[0].version, "2.42", "计划里是定版后的精确版本");
    assert_eq!(plan.packages[0].prefix, "opt/rt/binutils2.42", "独立前缀随计划走（装配按它挂载）");
    assert_eq!(plan.packages[2].kind, "system", "写进系统路径的包排在最后");
    assert_eq!(plan.base.as_deref(), Some("base-linux"));
    assert!(!plan.net, "默认不放行出站网络");
}

#[test]
fn diagnose_text_spells_out_every_reason() {
    let missing = exec::diagnose_text(&[Diagnosis::Missing { module: "a".to_string(), capability: "python".to_string() }]);
    assert!(missing.contains("模块 a") && missing.contains("python") && missing.contains("runtimes/"), "{}", missing);
    let ambiguous = exec::diagnose_text(&[Diagnosis::Ambiguous {
        capability: "python".to_string(),
        versions: vec!["3.11.9".to_string(), "3.12.4".to_string()],
    }]);
    assert!(ambiguous.contains("多个版本") && ambiguous.contains("3.12.4"), "{}", ambiguous);
    let bad = exec::diagnose_text(&[Diagnosis::UnknownPin { capability: "python".to_string(), version: "9.9".to_string() }]);
    assert!(bad.contains("定版 9.9"), "{}", bad);
    let clash = exec::diagnose_text(&[Diagnosis::Conflict {
        path: "usr/lib".to_string(),
        a: "a@1".to_string(),
        b: "b@1".to_string(),
    }]);
    assert!(clash.contains("usr/lib") && clash.contains("a@1") && clash.contains("b@1"), "{}", clash);
}

#[test]
fn runtime_report_is_tier_aware() {
    let cores = |pkgs: Arc<InMemoryPackages>, modules: Vec<Module>| {
        core_with_pkgs(
            modules,
            gw(BTreeMap::new(), vec!["[]".into()]),
            Arc::new(SilentRunner),
            Arc::new(FakeCatalog::new(vec!["m".to_string()])),
            Arc::new(InMemoryHistory::new()),
            Arc::new(InMemorySysIo::new()),
            pkgs,
        )
    };
    let core = cores(Arc::new(InMemoryPackages::empty()), vec![module_with_runtimes("a", &["python"])]);
    let host = core.runtime_report(Tier::Host);
    assert_eq!(host.tier, "host");
    assert_eq!(host.declared.get("a"), Some(&vec!["python".to_string()]));
    assert_eq!(host.missing.get("a"), Some(&vec!["python".to_string()]), "档位无关的事实照实报");
    assert!(host.available.is_empty());
    assert!(host.diagnoses.is_empty(), "本机档不做虚拟机档诊断");
    assert_eq!(core.runtime_report(Tier::Vm).diagnoses.len(), 1);
    // 包库里有包 = 缺失消失、诊断清空
    let with_pkg = cores(
        Arc::new(InMemoryPackages::with(&["id: python\nversion: 3.12.4\nprefix: opt/rt/python3.12"])),
        vec![module_with_runtimes("a", &["python"])],
    );
    let r = with_pkg.runtime_report(Tier::Vm);
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert_eq!(r.available.get("python"), Some(&vec!["3.12.4".to_string()]));
    assert!(r.diagnoses.is_empty(), "{:?}", r.diagnoses);
}

#[test]
fn module_without_runtime_is_denied_with_reason() {
    // 虚拟机档 + 空包库：模块声明的运行包没装载 → 工具不落进程，回执如实说缺哪个能力。
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![TOOL_CALL.into(), "{\"type\":\"say\",\"text\":\"改用内置工具\"}".into()]);
    let mut mod_a = module_with_runtimes("a", &["python"]);
    mod_a.manifest.tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut core = Core::new(
        Arc::new(InMemorySettings::with_tier(Tier::Vm)),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![mod_a])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gw(member, vec!["[]".into()])),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&runner) as Arc<dyn ToolRunner + Send + Sync>,
        Arc::new(InMemorySysIo::new()),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败");
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    assert!(runner.calls.lock().expect("锁").is_empty(), "缺运行包时不落进程");
    let texts = test_prompts().core.tool_texts;
    let expect = texts.render(
        &texts.module_unavailable,
        &[("module", "a".to_string()), ("capability", "python".to_string())],
    );
    let feedback = core
        .single_history(&sid)
        .unwrap()
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[工具结果] a.grep"))
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(feedback.contains(&expect), "回执要用册子文案：{}", feedback);
    let lines = tool_line_texts(&events);
    assert!(lines.iter().any(|l| l.contains("grep") && l.contains("失败")), "失败也要发 tool 行：{:?}", lines);
    // 同一个模块在本机档照旧执行（本机档不装载运行包）。
    let mut member2 = BTreeMap::new();
    member2.insert("a".to_string(), vec![TOOL_CALL.into(), "{\"type\":\"say\",\"text\":\"跑完了\"}".into()]);
    let mut mod_b = module_with_runtimes("a", &["python"]);
    mod_b.manifest.tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let runner2 = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "ok".into(), ok: true });
    let mut core2 = core_with_runner(vec![mod_b], gw(member2, vec!["[]".into()]), Arc::clone(&runner2));
    let sid2 = core2.create_work(work("w2", WorkMode::Single, &["a"])).unwrap().sid;
    with_live(|l| core2.single_say(&sid2, "干活", l)).unwrap();
    assert_eq!(runner2.calls.lock().expect("锁").len(), 1, "本机档不受包库影响");
}


// ---------- 配置视图：读、改、冻结 ----------

/// 造一条 agent 名单记录。
fn agent_meta(name: &str, modules: &[&str], model: Option<&str>) -> AgentMeta {
    AgentMeta {
        name: name.to_string(),
        transient: false,
        modules: modules.iter().map(|s| s.to_string()).collect(),
        model: model.map(|s| s.to_string()),
    }
}

/// 直接把一份 meta 放进内存历史（模拟"重启后从盘上读回该会话"）。
fn seed_session(hist: &Arc<InMemoryHistory>, name: &str, mode: &str, agents: Vec<AgentMeta>, exec: ExecSpec) {
    hist.create(&SessionMeta {
        name: name.to_string(),
        mode: mode.to_string(),
        delegate: false,
        modules: agents.iter().flat_map(|a| a.modules.clone()).collect(),
        task: None,
        ts: 1,
        agents,
        exec,
    })
    .unwrap();
}

/// 一次编辑提交（名字 / 模块 / 模型；档位与定版默认本机档）。
fn edit_of(agents: Vec<(&str, &[&str], &str)>) -> SessionEdit {
    SessionEdit {
        agents: agents
            .into_iter()
            .map(|(n, ms, m)| ConfigAgent {
                name: n.to_string(),
                modules: ms.iter().map(|s| s.to_string()).collect(),
                model: m.to_string(),
            })
            .collect(),
        tier: "host".to_string(),
        base: None,
        pins: BTreeMap::new(),
        net: false,
    }
}

#[test]
fn session_config_reports_tier_missing_and_runtimes_dir() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(&hist, "w", "single", vec![agent_meta("a", &["a"], Some("m"))], ExecSpec { tier: Tier::Vm, ..Default::default() });
    let core = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let cfg = core.session_config("w").unwrap();
    assert_eq!(cfg.sid, "w");
    assert_eq!(cfg.mode, "single");
    assert!(!cfg.started, "没有内容的会话 = 还没开过");
    assert_eq!(cfg.tier, "vm");
    assert_eq!(cfg.agents[0].model, "m");
    assert_eq!(cfg.runtime.missing.get("a"), Some(&vec!["python".to_string()]));
    assert!(cfg.runtimes_dir.ends_with("/runtimes"), "{}", cfg.runtimes_dir);
    assert!(core.session_config("没有这个会话").is_err());
}

#[test]
fn edit_session_writes_meta_appends_config_record_and_rebuilds() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut a = module_of("a");
    a.manifest.tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut core = core_with_pkgs(
        vec![a, module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let sid = core.create_work(work("w", WorkMode::Single, &["a"])).unwrap().sid;
    assert!(
        !core.session_config(&sid).unwrap().started,
        "单 agent 会话在用户开口之前还没内容：名字仍改得动"
    );
    // 改：模块 a → b，模型指定 m，档位换虚拟机档、放行网络。
    core.edit_session(
        &sid,
        SessionEdit {
            agents: vec![ConfigAgent { name: "a".to_string(), modules: vec!["b".to_string()], model: "m".to_string() }],
            tier: "vm".to_string(),
            base: Some("base-linux".to_string()),
            pins: BTreeMap::new(),
            net: true,
        },
    )
    .unwrap();
    let cfg = core.session_config(&sid).unwrap();
    assert_eq!(cfg.agents[0].modules, vec!["b".to_string()]);
    assert_eq!(cfg.tier, "vm");
    assert!(cfg.net, "网络开关随提交生效");
    let (meta, events) = hist.load(&sid).unwrap();
    assert_eq!(meta.agents[0].modules, vec!["b".to_string()], "meta.yaml 是名单的唯一真相");
    assert_eq!(meta.exec.tier, Tier::Vm);
    assert_eq!(meta.exec.base.as_deref(), Some("base-linux"));
    assert!(
        events.iter().any(|e| e.get("type").and_then(|t| t.as_str()) == Some("config")),
        "每次提交编辑追加一条旁路配置记录：{:?}",
        events
    );
    // 内存里的会话按旧配置装过：丢掉后下次访问按新配置从转录重建（内容不丢）。
    assert!(!core.session_exists(&sid));
    // 下一次访问按新配置从转录重建（单 agent 会话还没轮到用户：它只提醒，不硬发请求）。
    with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(core.session_exists(&sid), "访问会话即按新配置重建");
    let history = core.single_history(&sid).unwrap();
    assert!(!history.is_empty(), "重建后上下文还在");
}

#[test]
fn edit_session_freezes_names_only_after_content() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(&hist, "raw", "single", vec![agent_meta("a", &["a"], None)], ExecSpec::default());
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    // 没内容：名字与模块都能换。
    core.edit_session("raw", edit_of(vec![("新名", &["b"], "")])).unwrap();
    assert_eq!(core.session_config("raw").unwrap().agents[0].name, "新名");
    // 有内容之后：名字冻结，模块与模型照旧可改。
    hist.append("raw", &[serde_json::json!({"type":"transcript","lines":[{"id":0,"line":"[用户] 你好"}]})])
        .unwrap();
    let err = core.edit_session("raw", edit_of(vec![("再改", &["b"], "")])).unwrap_err();
    assert!(err.contains("冻结"), "{}", err);
    core.edit_session("raw", edit_of(vec![("新名", &["a"], "m")])).unwrap();
    assert_eq!(core.session_config("raw").unwrap().agents[0].modules, vec!["a".to_string()]);
}

#[test]
fn edit_session_enforces_the_same_rules_as_creation() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(&hist, "w", "single", vec![agent_meta("a", &["a"], None)], ExecSpec::default());
    seed_session(
        &hist,
        "c",
        "collab",
        vec![agent_meta("a", &["a"], None), agent_meta("b", &["b"], None)],
        ExecSpec::default(),
    );
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let e = core.edit_session("w", edit_of(vec![("a", &["没有这个模块"], "")])).unwrap_err();
    assert!(e.contains("无此模块"), "{}", e);
    let e = core.edit_session("c", edit_of(vec![("x", &["a"], ""), ("y", &["a"], "")])).unwrap_err();
    assert!(e.contains("只能属于一个 agent"), "{}", e);
    let mut bad_tier = edit_of(vec![("x", &["a"], "")]);
    bad_tier.tier = "docker".to_string();
    assert!(core.edit_session("c", bad_tier).unwrap_err().contains("未知执行档位"));
    let mut two_agents = edit_of(vec![("x", &["a"], ""), ("y", &["b"], "")]);
    two_agents.tier = "host".to_string();
    assert!(core.edit_session("w", two_agents).unwrap_err().contains("只接受一个 agent"));

    // 虚拟机档选型不成立（同一能力多版本未定版）：编辑与「开始」同一把尺子，如实拒绝。
    let hist2 = Arc::new(InMemoryHistory::new());
    seed_session(&hist2, "c", "collab", vec![agent_meta("x", &["a"], None)], ExecSpec::default());
    let mut core2 = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist2),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::with(&["id: python\nversion: 3.12.4\nprefix: opt/rt/py312", "id: python\nversion: 3.11.9\nprefix: opt/rt/py311"])),
    );
    let mut vm_edit = edit_of(vec![("x", &["a"], "")]);
    vm_edit.tier = "vm".to_string();
    let e = core2.edit_session("c", vm_edit.clone()).unwrap_err();
    assert!(e.contains("多个版本"), "{}", e);
    // 定版之后可以提交（缺包不拦：那只是该模块工具不可用）。
    vm_edit.pins = BTreeMap::from([("python".to_string(), "3.12.4".to_string())]);
    core2.edit_session("c", vm_edit).unwrap();
    assert_eq!(core2.session_config("c").unwrap().pins.get("python").map(String::as_str), Some("3.12.4"));
}

#[test]
fn deleting_a_session_asks_the_fence_to_release_its_grants() {
    let hist = Arc::new(InMemoryHistory::new());
    let fence = Arc::new(RecordingFence::new());
    seed_session(&hist, "w", "single", vec![agent_meta("甲", &["a"], None)], ExecSpec::default());
    let mut core = Core::new(
        Arc::new(InMemorySettings::new()),
        Arc::clone(&hist) as Arc<dyn crate::core::ports::HistoryStore + Send + Sync>,
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::clone(&fence) as Arc<dyn crate::core::ports::FenceHost + Send + Sync>,
        Arc::new(gw(BTreeMap::new(), vec!["[]".into()])),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败");
    assert!(core.history_delete("w").unwrap(), "会话目录该被删掉");
    assert_eq!(
        fence.released.lock().expect("锁").as_slice(),
        &["甲".to_string()],
        "删除会话要先请适配层撤销该 agent 的围栏授权（痕迹与会话同生共死）"
    );
}

#[test]
fn session_meta_exec_section_roundtrips_and_reads_legacy_meta() {
    let meta = SessionMeta {
        name: "w".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 1,
        agents: Vec::new(),
        exec: ExecSpec {
            tier: Tier::Vm,
            base: Some("base-linux".to_string()),
            pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
            net: false,
        },
    };
    let text = serde_yaml::to_string(&meta).expect("序列化");
    let back: SessionMeta = serde_yaml::from_str(&text).expect("反序列化");
    assert_eq!(back.exec.tier, Tier::Vm);
    assert_eq!(back.exec.base.as_deref(), Some("base-linux"));
    assert_eq!(back.exec.pins.get("python").map(String::as_str), Some("3.12.4"));
    assert!(!back.exec.net);
    // 缺 exec 段的旧会话照旧可读（默认 = 本机档、不联网、不定版）。
    let legacy: SessionMeta = serde_yaml::from_str("name: old\nmode: single\nmodules: [a]\nts: 1\n").expect("旧 meta.yaml 必须可读");
    assert_eq!(legacy.exec.tier, Tier::Host);
    assert!(legacy.exec.base.is_none());
    assert!(!legacy.exec.net);
    assert!(legacy.exec.pins.is_empty());
}

