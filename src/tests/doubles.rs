//! 测试替身与测试组合根（doubles）：内存适配器 + 共享夹具 + 装配辅助。
//! 语义规范见 docs/testing/doubles.md；端口契约测试与它同处一层（src/tests/）。
//! 测试里的组合根 = 内存适配器；core 的可测性正是端口化的直接收益。
//!
//! 本模块只放**替身与装配辅助**；用它们的用例在 `super::core`（T1）。所以这里对"只有用例才用到"的项
//! 放行 dead_code（替身与夹具是给别的模块用的，不在本文件里被调用是正常的）。
#![allow(dead_code)]

use super::core::SilentRunner;
use crate::adapters::fake_chat::FakeChat;
use crate::core::events::Live;
use crate::core::exec::Tier;
use crate::core::history::{HistoryView, SessionMeta};
use crate::core::module::{Module, ModuleManifest};
use crate::core::packages::{Library, PackageManifest};
use crate::core::ports::{
    BoxedChat, Chat, ChatGateway, Chunk, CompleteOpts, Completion, FileRead, HistoryStore,
    ModelCatalog, ModuleSource, Msg, PackageSource, PromptSource, SettingsStore, SysIo, ToolRunner,
    Workspace,
};
use crate::core::prompt::Prompts;
use crate::core::providers::{Channel, ModelEntry, Provider, Settings};
use crate::core::{AgentInstance, Core, SessionEvent, WorkMode, WorkSpec};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
            Provider {
                base_url: "http://test".to_string(),
                api_key: "k".to_string(),
            },
        );
        s.models.insert(
            "m".to_string(),
            ModelEntry {
                name: "M".to_string(),
                api_model: "m".to_string(),
                provider: "p".to_string(),
                note: String::new(),
                tools: crate::core::providers::ToolMode::Envelope,
            },
        );
        s.core = Some("m".to_string());
        InMemorySettings {
            s: Mutex::new(s),
            fail: None,
        }
    }

    /// 注入失败：load / save 一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemorySettings {
        self.fail = Some(msg.to_string());
        self
    }
    /// 指定默认执行档位的登记处（断言虚拟机档下的诊断与工具回执）。
    pub(crate) fn with_tier(tier: Tier) -> InMemorySettings {
        let s = InMemorySettings::new();
        s.s.lock().expect("锁").app.tier = tier;
        s
    }

    /// 指定全局流式与调用预算的登记处（断言"设置是流式的上限、预算全局通用"）。
    pub(crate) fn with_llm(streaming: bool, timeout_secs: u64) -> InMemorySettings {
        let s = InMemorySettings::new();
        {
            let mut g = s.s.lock().expect("锁");
            g.app.streaming = streaming;
            g.app.llm_timeout_secs = timeout_secs;
        }
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
        InMemoryWorkspace {
            files: Mutex::new(BTreeMap::new()),
            fail: None,
        }
    }

    /// 注入失败：prepare / roots / write_work / list 一律返回该原因（work_has 是布尔查询，不受影响）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryWorkspace {
        self.fail = Some(msg.to_string());
        self
    }
    /// 直接放一个文件（模拟落盘），键与真实布局同构：<session>/work/<名字> 或 <session>/<agent>/<相对路径>。
    pub(crate) fn seed(&self, session: &str, area: &str, rel: &str) {
        self.files
            .lock()
            .expect("锁")
            .insert(format!("{}/{}/{}", session, area, rel), Vec::new());
    }
}

impl Workspace for InMemoryWorkspace {
    fn prepare(&self, _session: &str, _agents: &[String]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(())
    }
    fn roots(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::core::workspace::WorkRoots, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 与 FsWorkspace 同构的**绝对**路径（以当前目录为锚；只读 env，不碰盘）。
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), abs(&[session, a]));
        }
        Ok(crate::core::workspace::WorkRoots {
            shared: abs(&[session, "work"]),
            agents: map,
        })
    }
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files
            .lock()
            .expect("锁")
            .insert(format!("{}/work/{}", session, name), bytes.to_vec());
        Ok(())
    }
    fn work_has(&self, session: &str, name: &str) -> bool {
        self.files
            .lock()
            .expect("锁")
            .contains_key(&format!("{}/work/{}", session, name))
    }
    fn list(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::core::workspace::WorkFiles, String> {
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
    /// 读取标注：模拟"超过单次上限"与"含非法 UTF-8"的文件（工具必须如实标注，而不是照改）。
    lossy: bool,
    cut: bool,
    /// 并发观测：**同时在读**的调用数与其峰值——并发用例据此断言"真的并发"（串行绝不会重叠）。
    active: AtomicUsize,
    peak: AtomicUsize,
    /// 每次读取的固定耗时（毫秒）：给并发留出可观测的窗口。
    delay_ms: u64,
    /// 指定文件的额外耗时：用来构造"后发的先完成"，验结果仍按原始顺序回填。
    extra: Mutex<BTreeMap<String, u64>>,
}

impl InMemorySysIo {
    pub(crate) fn new() -> InMemorySysIo {
        InMemorySysIo {
            files: Mutex::new(BTreeMap::new()),
            fail: None,
            lossy: false,
            cut: false,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay_ms: 0,
            extra: Mutex::new(BTreeMap::new()),
        }
    }

    /// 注入读取标注（lossy = 含非法 UTF-8；cut = 只读到开头）。
    pub(crate) fn marked(mut self, lossy: bool, cut: bool) -> InMemorySysIo {
        self.lossy = lossy;
        self.cut = cut;
        self
    }

    /// 注入失败：read / write 一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemorySysIo {
        self.fail = Some(msg.to_string());
        self
    }

    /// 让每次读取慢一点：并发（峰值 > 1）与串行（峰值恒为 1）因此可观测。
    pub(crate) fn slow(mut self, ms: u64) -> InMemorySysIo {
        self.delay_ms = ms;
        self
    }

    /// 指定文件每读一次额外慢多少毫秒。
    pub(crate) fn slow_file(&self, parts: &[&str], ms: u64) {
        self.extra.lock().expect("锁").insert(p(parts), ms);
    }

    /// 同时在读的峰值（并发用例的唯一证据）。
    pub(crate) fn peak_concurrent_reads(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    /// 记一次"进入读取"：整个读取期间计数 +1，并如实记下峰值与耗时。
    fn enter(&self, key: &str) {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let extra = self
            .extra
            .lock()
            .expect("锁")
            .get(key)
            .copied()
            .unwrap_or(0);
        let ms = self.delay_ms + extra;
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
    pub(crate) fn seed(&self, parts: &[&str], text: &str) {
        self.files
            .lock()
            .expect("锁")
            .insert(p(parts), text.to_string());
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
        self.enter(&key);
        let text = self
            .files
            .lock()
            .expect("锁")
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("读取失败：{} 不存在", key))?;
        Ok(FileRead {
            bytes: text.len(),
            text,
            lossy: self.lossy,
            cut: self.cut,
        })
    }
    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files
            .lock()
            .expect("锁")
            .insert(path.to_string_lossy().into_owned(), content.to_string());
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

/// 什么都不修的修复端口（严格要求合法信封）：测试基线，也是"宁缺毋滥"部署的对照实现。
pub(crate) struct NoRepair;

impl crate::core::ports::EnvelopeRepair for NoRepair {
    fn repair(
        &self,
        _raw: &str,
        _kind: &crate::core::envelope::Malformed,
    ) -> crate::core::ports::RepairOutcome {
        crate::core::ports::RepairOutcome {
            repaired: None,
            what: Vec::new(),
        }
    }
}

/// 一次内置工具调用（空账本）：只关心工具行为本身的用例用它；
/// 关心"改动前有没有读过"的用例直接用 systool::execute 并自带 Observations。
pub(crate) fn run_builtin(
    sb: &crate::core::workspace::Sandbox,
    io: &dyn crate::core::ports::SysIo,
    name: &str,
    args_json: &str,
) -> crate::core::ports::ToolOutcome {
    let mut obs = crate::core::systool::Observations::default();
    crate::core::systool::execute(sb, io, &mut obs, name, args_json)
}

/// 测试用模块工具声明：只给启动命令（参数契约在需要的用例里另行声明）。
pub(crate) fn decl(command: &str) -> crate::core::module::ToolDecl {
    crate::core::module::ToolDecl {
        command: command.to_string(),
        desc: String::new(),
        params: None,
        parallel: false,
    }
}

/// 测试用模块工具声明：带参数契约（YAML 里的 params 段）。
pub(crate) fn decl_with(command: &str, params_yaml: &str) -> crate::core::module::ToolDecl {
    let mut d = decl(command);
    d.params = Some(serde_yaml::from_str(params_yaml).expect("测试参数声明要能解析"));
    d
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
        builtin_tools: test_prompts().core.builtin_tools,
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
        InMemoryHistory {
            metas: Mutex::new(BTreeMap::new()),
            events: Mutex::new(BTreeMap::new()),
            fail: None,
        }
    }

    /// 注入失败：全部 HistoryStore 方法一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryHistory {
        self.fail = Some(msg.to_string());
        self
    }

    /// 直接改掉某条会话已落盘的 meta（测试夹具）：用来构造"落盘档位与当前判据不一致"的情形。
    /// 例如虚拟机档现在一律不可选，但**已存在的**虚拟机档会话必须还能打开（记录是用户的）。
    pub(crate) fn force_tier(&self, name: &str, tier: crate::core::exec::Tier) {
        let mut metas = self.metas.lock().expect("锁");
        if let Some(m) = metas.get_mut(name) {
            m.exec.tier = tier;
        }
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
        self.metas
            .lock()
            .expect("锁")
            .insert(meta.name.clone(), meta.clone());
        Ok(())
    }
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        self.guard()?;
        self.metas
            .lock()
            .expect("锁")
            .insert(meta.name.clone(), meta.clone());
        Ok(())
    }
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.guard()?;
        self.events
            .lock()
            .expect("锁")
            .entry(name.to_string())
            .or_default()
            .extend_from_slice(events);
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
                    .map(|v| {
                        v.iter()
                            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("ended"))
                    })
                    .unwrap_or(false),
                exec: m.exec.clone(),
            })
            .collect())
    }
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        self.guard()?;
        let meta = self
            .metas
            .lock()
            .expect("锁")
            .get(name)
            .cloned()
            .ok_or_else(|| format!("无此会话：{}", name))?;
        let events = self
            .events
            .lock()
            .expect("锁")
            .get(name)
            .cloned()
            .unwrap_or_default();
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
        FakeCatalog {
            models,
            seen: Mutex::new(Vec::new()),
            fail: None,
        }
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
        crate::core::module::Roster {
            modules: self.0.clone(),
            rejected: Vec::new(),
        }
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
        RecordingFence {
            released: Mutex::new(Vec::new()),
            fail: None,
        }
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
    pkg_yaml(&format!(
        "id: {}
version: {}
prefix: opt/rt/{}-{}",
        id, version, id, version
    ))
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
pub(crate) fn module_with_runtimes(id: &str, caps: &[&str]) -> Module {
    let mut m = module_of(id);
    m.manifest.runtimes = caps.iter().map(|s| s.to_string()).collect();
    m
}

/// 测试用静默 Live（不流式、不派发短暂事件）。
pub(crate) fn with_live<T>(f: impl FnOnce(&mut Live) -> T) -> T {
    let mut noop = |_e: SessionEvent| {};
    let mut live = Live {
        llm: Default::default(),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        emit: &mut noop,
    };
    f(&mut live)
}

/// 把模块包成"每模块一个临时 agent"（协作的成员语义）。
pub(crate) fn agents_of(modules: &[&str]) -> Vec<AgentInstance> {
    modules
        .iter()
        .map(|m| AgentInstance {
            name: m.to_string(),
            transient: true,
            modules: vec![m.to_string()],
            model: None,
        })
        .collect()
}

/// 测试用工作规格（模型留空 = 走核心默认 m）。
/// 单 agent 形态：1 个 agent（模块数不限；单模块时以模块名给 agent 起名，便于断言说话人）；
/// 协作形态：每模块一个 agent（成员 id = agent 名 = 模块名）。
pub(crate) fn work(name: &str, mode: WorkMode, modules: &[&str]) -> WorkSpec {
    let agents = if mode == WorkMode::Collab {
        agents_of(modules)
    } else {
        let agent_name = if modules.len() == 1 {
            modules[0]
        } else {
            "组合"
        };
        vec![AgentInstance {
            name: agent_name.to_string(),
            transient: true,
            modules: modules.iter().map(|s| s.to_string()).collect(),
            model: None,
        }]
    };
    WorkSpec {
        name: name.to_string(),
        mode,
        agents,
        task: None,
        delegate: false,
    }
}

/// 协作工作规格（含需求）。
pub(crate) fn collab_work(name: &str, modules: &[&str], delegate: bool, task: &str) -> WorkSpec {
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
    fn complete(
        &mut self,
        _messages: &[Msg],
        _opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        let mut q = self.q.lock().expect("脚本队列锁");
        let text = if q.len() > 1 {
            q.remove(0)
        } else {
            q.first().cloned().unwrap_or_default()
        };
        Completion::text(text)
    }
}

/// 脚本网关：按 agent 实例名回放各自脚本；核心通道走共享队列。
pub(crate) struct ScriptGateway {
    member: BTreeMap<String, Vec<String>>,
    core: Arc<Mutex<Vec<String>>>,
}
impl ScriptGateway {
    pub(crate) fn new(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
        ScriptGateway {
            member,
            core: Arc::new(Mutex::new(core)),
        }
    }
}

impl ChatGateway for ScriptGateway {
    fn probe_tools(&self, _c: &Channel) -> Result<crate::core::ports::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(&self, _c: Option<&Channel>, id: &str) -> (BoxedChat, Option<String>) {
        let script =
            self.member.get(id).cloned().unwrap_or_else(|| {
                vec!["{\"type\":\"say\",\"text\":\"（演示）收到。\"}".to_string()]
            });
        (scripted(script), None)
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (
            Box::new(SharedScript {
                q: Arc::clone(&self.core),
            }),
            false,
        )
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
    serde_yaml::from_str::<Prompts>(include_str!("../../prompts.yaml"))
        .expect("内置提示词册必须合法")
}

pub(crate) fn core_with(modules: Vec<Module>, gateway: ScriptGateway) -> Core {
    core_with_runner(modules, gateway, Arc::new(SilentRunner))
}

/// 注入指定内存工作区的装配（断言 @ 文件清单取自会话 meta.agents）。
pub(crate) fn core_with_workspace(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    ws: Arc<InMemoryWorkspace>,
) -> Core {
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
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

/// 注入指定内存文件系统的装配（断言内置文件工具真正落盘）。
pub(crate) fn core_with_io(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    io: Arc<InMemorySysIo>,
) -> Core {
    core_with_all(
        modules,
        gateway,
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(InMemoryHistory::new()),
        io,
    )
}

pub(crate) fn core_with_runner(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
) -> Core {
    core_with_catalog(
        modules,
        gateway,
        runner,
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    )
}

/// 指定模型目录的装配（断言编辑期密钥复用）。
pub(crate) fn core_with_catalog(
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
pub(crate) fn core_with_all(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Core {
    core_with_pkgs(
        modules,
        gateway,
        runner,
        catalog,
        history,
        io,
        Arc::new(InMemoryPackages::empty()),
    )
}

/// 指定全部端口 + 运行包库的装配（断言缺包诊断与工具回执）。
pub(crate) fn core_with_pkgs(
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
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

/// 用**指定登记处**装配（断言"全局设置是流式的上限、预算全局通用"这类判据）。
pub(crate) fn core_with_settings(store: InMemorySettings) -> Core {
    Core::new(
        Arc::new(store),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(Vec::new())),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gw(BTreeMap::new(), Vec::new())),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

pub(crate) fn gw(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
    ScriptGateway {
        member,
        core: Arc::new(Mutex::new(core)),
    }
}

/// 指定任意网关 + 指定内存文件系统的装配（既换通道又要断言落盘的用例用它）。
pub(crate) fn core_with_io_gateway(
    modules: Vec<Module>,
    gateway: impl ChatGateway + Send + Sync + 'static,
    io: Arc<InMemorySysIo>,
) -> Core {
    Core::new(
        Arc::new(InMemorySettings::new()),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(modules)),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        io,
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

/// 指定任意网关的装配（入站契约测试用：需要自定义时序的通道）。
pub(crate) fn core_with_gateway(
    modules: Vec<Module>,
    gateway: impl ChatGateway + Send + Sync + 'static,
) -> Core {
    Core::new(
        Arc::new(InMemorySettings::new()),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(modules)),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}
