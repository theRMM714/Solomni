//! 测试替身与测试组合根（doubles）：内存适配器 + 共享夹具 + 装配辅助。
//! 语义规范见 docs/testing/doubles.md；端口契约测试与它同处一层（src/tests/）。
//! 测试里的组合根 = 内存适配器；conductor 的可测性正是端口化的直接收益。
//!
//! 本模块只放**替身与装配辅助**；用它们的用例在各业务测试文件（T1，见 `src/tests/`）。所以这里对
//! "只有用例才用到"的项放行 dead_code（替身与夹具是给别的模块用的，不在本文件里被调用是正常的）。
#![allow(dead_code)]

use super::builders::SilentRunner;
use crate::capabilities::conductor::api::{AgentInstance, SessionEvent, WorkMode, WorkSpec};
use crate::capabilities::conductor::service::Conductor;
use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};
use crate::capabilities::llm::detail::fake_chat::FakeChat;
use crate::capabilities::llm::ports::{ChatGateway, ModelCatalog};
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::prompt::domain::prompt::Prompts;
use crate::capabilities::prompt::ports::PromptSource;
use crate::capabilities::registry::api::{ModelEntry, Provider, Settings};
use crate::capabilities::registry::ports::SettingsStore;
use crate::capabilities::session::api::Live;
use crate::capabilities::session::api::{HistoryView, SessionMeta};
use crate::capabilities::session::ports::HistoryStore;
use crate::capabilities::tools::ports::SystoolsSource;
use crate::capabilities::tools::ports::{FileRead, SysIo};
use crate::capabilities::workspace::api::{Library, PackageManifest};
use crate::capabilities::workspace::api::{Module, ModuleManifest};
use crate::capabilities::workspace::ports::{ModuleSource, PackageSource, WorkStore, Workdirs};
use crate::kernel::api::Tier;
use crate::kernel::ports::ProcessRunner;
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
                tools: crate::capabilities::llm::api::ToolMode::Envelope,
                context: 32_000,
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

    /// 放一个**指定大小**的文件（用量断言用）：键与真实布局同构。
    pub(crate) fn seed_bytes(&self, session: &str, area: &str, rel: &str, bytes: usize) {
        self.files
            .lock()
            .expect("锁")
            .insert(format!("{}/{}/{}", session, area, rel), vec![0u8; bytes]);
    }
}

impl Workdirs for InMemoryWorkspace {
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
    ) -> Result<crate::capabilities::workspace::api::WorkRoots, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 与 FsWorkspace 同构的**绝对**路径（以当前目录为锚；只读 env，不碰盘）。
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), abs(&[session, a]));
        }
        Ok(crate::capabilities::workspace::api::WorkRoots {
            shared: abs(&[session, "work"]),
            agents: map,
            store: abs(&[session, ".work"]),
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
    ) -> Result<crate::capabilities::workspace::api::WorkFiles, String> {
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
        Ok(crate::capabilities::workspace::api::WorkFiles { work, agents: map })
    }

    fn usage(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::capabilities::workspace::api::WorkUsage, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        use crate::capabilities::workspace::api::AreaUsage;
        let files = self.files.lock().expect("锁");
        let area = |prefix: &str| -> AreaUsage {
            let mut out = AreaUsage::default();
            for (k, v) in files.iter() {
                if k.starts_with(prefix) {
                    out.files += 1;
                    out.bytes += v.len() as u64;
                }
            }
            out
        };
        let work = area(&format!("{}/work/", session));
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), area(&format!("{}/{}/", session, a)));
        }
        Ok(crate::capabilities::workspace::api::WorkUsage::total(
            work, map,
        ))
    }
}

/// 内存版本库：文件原语 + 内容寻址对象、提交记录、head 与各 agent 的拉取基线。
/// 键一律用「根路径字符串|相对键」，与真实适配器同一份语义，供端口契约测试与协调业务用例使用。
#[derive(Default)]
pub(crate) struct InMemoryWorkStore {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    commits: Mutex<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Commit>>,
    heads: Mutex<BTreeMap<String, u64>>,
    index: Mutex<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Index>>,
    fail: Option<String>,
}

fn ws_key(root: &std::path::Path, rel: &str) -> String {
    format!("{}|{}", root.to_string_lossy(), rel)
}

impl InMemoryWorkStore {
    pub(crate) fn new() -> InMemoryWorkStore {
        InMemoryWorkStore::default()
    }

    /// 注入失败：任何端口调用都返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryWorkStore {
        self.fail = Some(msg.to_string());
        self
    }

    /// 直接放一个文件（模拟主副本 / 沙箱里已有内容）。
    pub(crate) fn seed(&self, root: &std::path::Path, rel: &str, text: &str) {
        self.files
            .lock()
            .expect("锁")
            .insert(ws_key(root, rel), text.as_bytes().to_vec());
    }
}

impl WorkStore for InMemoryWorkStore {
    fn read_under(&self, root: &std::path::Path, rel: &str) -> Result<Option<Vec<u8>>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .files
            .lock()
            .expect("锁")
            .get(&ws_key(root, rel))
            .cloned())
    }

    fn write_under(&self, root: &std::path::Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files
            .lock()
            .expect("锁")
            .insert(ws_key(root, rel), bytes.to_vec());
        Ok(())
    }

    fn remove_under(&self, root: &std::path::Path, rel: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files.lock().expect("锁").remove(&ws_key(root, rel));
        Ok(())
    }

    fn list(&self, root: &std::path::Path) -> Result<Vec<String>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", root.to_string_lossy());
        let files = self.files.lock().expect("锁");
        let mut out: Vec<String> = files
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix).map(|s| s.to_string()))
            .collect();
        out.sort();
        Ok(out)
    }

    fn head(&self, store: &std::path::Path) -> Result<Option<u64>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .heads
            .lock()
            .expect("锁")
            .get(&store.to_string_lossy().to_string())
            .copied())
    }

    fn set_head(&self, store: &std::path::Path, id: u64) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.heads
            .lock()
            .expect("锁")
            .insert(store.to_string_lossy().to_string(), id);
        Ok(())
    }

    fn clear_head(&self, store: &std::path::Path) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.heads
            .lock()
            .expect("锁")
            .remove(&store.to_string_lossy().to_string());
        Ok(())
    }

    fn read_commit(
        &self,
        store: &std::path::Path,
        id: u64,
    ) -> Result<Option<crate::capabilities::workspace::domain::workstore::Commit>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .commits
            .lock()
            .expect("锁")
            .get(&ws_key(store, &id.to_string()))
            .cloned())
    }

    fn write_commit(
        &self,
        store: &std::path::Path,
        commit: &crate::capabilities::workspace::domain::workstore::Commit,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.commits
            .lock()
            .expect("锁")
            .insert(ws_key(store, &commit.id.to_string()), commit.clone());
        Ok(())
    }

    fn list_commits(&self, store: &std::path::Path) -> Result<Vec<u64>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", store.to_string_lossy());
        let commits = self.commits.lock().expect("锁");
        let mut ids: Vec<u64> = commits
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix)?.parse::<u64>().ok())
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }

    fn remove_commit(&self, store: &std::path::Path, id: u64) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.commits
            .lock()
            .expect("锁")
            .remove(&ws_key(store, &id.to_string()));
        Ok(())
    }

    fn read_index(
        &self,
        store: &std::path::Path,
        agent: &str,
    ) -> Result<crate::capabilities::workspace::domain::workstore::Index, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .index
            .lock()
            .expect("锁")
            .get(&ws_key(store, agent))
            .cloned()
            .unwrap_or_default())
    }

    fn write_index(
        &self,
        store: &std::path::Path,
        agent: &str,
        index: &crate::capabilities::workspace::domain::workstore::Index,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.index
            .lock()
            .expect("锁")
            .insert(ws_key(store, agent), index.clone());
        Ok(())
    }

    fn write_object(
        &self,
        store: &std::path::Path,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.objects
            .lock()
            .expect("锁")
            .entry(ws_key(store, hash))
            .or_insert_with(|| bytes.to_vec());
        Ok(())
    }

    fn read_object(&self, store: &std::path::Path, hash: &str) -> Result<Vec<u8>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.objects
            .lock()
            .expect("锁")
            .get(&ws_key(store, hash))
            .cloned()
            .ok_or_else(|| format!("对象不存在：{}", hash))
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
    fn list(
        &self,
        path: &std::path::Path,
    ) -> Result<Vec<crate::capabilities::tools::ports::DirEntry>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 内存替身：把已 seed 的路径按"父目录等于该目录"筛出来（只报直接子项）。
        let dir = path.to_string_lossy().into_owned();
        let files = self.files.lock().expect("锁");
        let mut out: Vec<crate::capabilities::tools::ports::DirEntry> = Vec::new();
        for (k, v) in files.iter() {
            let Some((parent, name)) = k.rsplit_once(['/', '\\']) else {
                continue;
            };
            if parent != dir.trim_end_matches(['/', '\\']) {
                continue;
            }
            out.push(crate::capabilities::tools::ports::DirEntry {
                name: name.to_string(),
                is_dir: false,
                bytes: v.len() as u64,
            });
        }
        if out.is_empty() {
            return Err(format!("列目录失败：{} 不是目录", dir));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
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

impl crate::capabilities::llm::ports::EnvelopeRepair for NoRepair {
    fn repair(
        &self,
        _raw: &str,
        _kind: &crate::capabilities::llm::api::Malformed,
    ) -> crate::capabilities::llm::api::RepairOutcome {
        crate::capabilities::llm::api::RepairOutcome {
            repaired: None,
            what: Vec::new(),
        }
    }
}

/// 一次内置工具调用（空账本）：只关心工具行为本身的用例用它；
/// 关心"改动前有没有读过"的用例自带 Observations 再调它。
/// 这里直接走 tools 的 domain（测试层允许）：生产路径是 `ToolExec::run_builtin`。
pub(crate) fn run_builtin(
    sb: &crate::capabilities::workspace::api::Sandbox,
    io: &dyn crate::capabilities::tools::ports::SysIo,
    name: &str,
    args_json: &str,
) -> crate::kernel::api::ToolOutcome {
    let mut obs = crate::capabilities::tools::api::Observations::default();
    crate::capabilities::tools::service::systool::execute(
        sb,
        &test_systools().tools,
        io,
        &mut obs,
        name,
        args_json,
    )
}

/// 测试用模块工具声明：只给启动命令（参数契约在需要的用例里另行声明）。
pub(crate) fn decl(command: &str) -> crate::capabilities::workspace::domain::module::ToolDecl {
    crate::capabilities::workspace::domain::module::ToolDecl {
        command: command.to_string(),
        desc: String::new(),
        params: None,
        parallel: false,
    }
}

/// 测试用模块工具声明：带参数契约（YAML 里的 params 段）。
pub(crate) fn decl_with(
    command: &str,
    params_yaml: &str,
) -> crate::capabilities::workspace::domain::module::ToolDecl {
    let mut d = decl(command);
    d.params = Some(yaml_serde::from_str(params_yaml).expect("测试参数声明要能解析"));
    d
}

/// 测试沙箱：work 共享区 + agent 私有区 + 指定模块目录（都是绝对路径）。
pub(crate) fn test_sandbox(
    agent: &str,
    modules: &[&str],
) -> crate::capabilities::workspace::api::Sandbox {
    let mut map = BTreeMap::new();
    for id in modules {
        map.insert(id.to_string(), abs(&["mods", id]));
    }
    crate::capabilities::workspace::api::Sandbox {
        work_name: "demo".to_string(),
        session: "demo".to_string(),
        agent: agent.to_string(),
        // 测试助手默认**可写共享区**：内置文件工具的既有用例直接写 work/ 复核行为；
        // 生产里 agent 会话是只读（env.rs 设 false），只读语义由 test_sandbox_readonly 单独钉。
        shared_writable: true,
        shared: abs(&["demo", "work"]),
        private: abs(&["demo", agent]),
        modules: map,
        modules_with_userdata: std::collections::BTreeSet::new(),
        permissions: Default::default(),
        texts: test_prompts().tools(),
    }
}

/// 只读共享区的测试沙箱：模拟**生产里 agent 会话**的默认（共享主副本只读，写入走 work_commit）。
pub(crate) fn test_sandbox_readonly(
    agent: &str,
    modules: &[&str],
) -> crate::capabilities::workspace::api::Sandbox {
    let mut sb = test_sandbox(agent, modules);
    sb.shared_writable = false;
    sb
}

/// 测试用会话参数：agent 名 + 该 agent 的沙箱（无模块）。身份块由它现渲染。
pub(crate) fn test_params(agent: &str) -> crate::capabilities::session::api::SessionParams {
    crate::capabilities::session::api::SessionParams::from_workspace(
        agent,
        &test_sandbox(agent, &[]),
        &[],
        "single",
        "solo",
    )
}

/// 测试用工具说明块素材（patch 语法 + 给定的模块工具）。
pub(crate) fn test_notes(
    sb: &crate::capabilities::workspace::api::Sandbox,
    modules: &[crate::capabilities::workspace::api::Module],
) -> crate::capabilities::tools::api::ToolNotes {
    crate::capabilities::tools::api::tool_notes(&test_prompts(), sb, modules)
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
    pub(crate) fn force_tier(&self, name: &str, tier: crate::kernel::api::Tier) {
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
    fn replace(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        self.guard()?;
        self.events
            .lock()
            .expect("锁")
            .insert(name.to_string(), events.to_vec());
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
                parent: m.parent.clone(),
                run: m.run,
            })
            .collect())
    }
    fn meta(&self, name: &str) -> Result<SessionMeta, String> {
        self.guard()?;
        self.metas
            .lock()
            .expect("锁")
            .get(name)
            .cloned()
            .ok_or_else(|| format!("无此会话：{}", name))
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
    pub(crate) seen: Mutex<Vec<String>>,
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
    fn list_models(&self, base_url: &str, _api_key: &str) -> Result<Vec<String>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.seen.lock().expect("锁").push(base_url.to_string());
        Ok(self.models.clone())
    }
}

pub(crate) struct VecSource(pub(crate) Vec<Module>);

impl ModuleSource for VecSource {
    fn scan(&self) -> crate::capabilities::workspace::api::Roster {
        crate::capabilities::workspace::api::Roster {
            modules: self.0.clone(),
            rejected: Vec::new(),
        }
    }
}

/// 无声围栏端口：测试里不碰任何 ACL（真实实现在 capabilities/tools/detail/confine）。
pub(crate) struct NoFenceHost;
impl crate::kernel::ports::FenceHost for NoFenceHost {
    fn release(&self, _spec: &crate::kernel::api::FenceSpec) -> Result<(), String> {
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
impl crate::kernel::ports::FenceHost for RecordingFence {
    fn release(&self, spec: &crate::kernel::api::FenceSpec) -> Result<(), String> {
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
    yaml_serde::from_str(y).expect("包清单必须能解析")
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
            services: Default::default(),
            secrets: Default::default(),
        },
        root: abs(&[id]),
        has_userdata: false,
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
        decisions: None,
        ask: None,
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
        tier: crate::kernel::api::Tier::Host,
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
    fn probe_tools(
        &self,
        _c: &Channel,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
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
    // 走**与产品同一条**装配路径（目录 + 合并）：替身与真机装配出同一册子，测试才有意义。
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate::capabilities::prompt::detail::yaml_prompts::YamlPrompts::new(root.join("prompts"))
        .load()
        .expect("内置提示词册必须合法")
}

/// 测试用工具总表与角色表（走**与产品同一条**装配路径）。
/// 它与提示词册**分开**装配：两者互不依赖（见 capabilities/prompt 的 Prompts）。
/// 测试用的**会话历史面**（与生产同一条路：端口装进 `SessionService`）。
pub(crate) fn test_history() -> Arc<dyn crate::capabilities::session::api::History + Send + Sync> {
    test_history_of(Arc::new(InMemoryHistory::new()))
}

/// 同上，但用调用方给的替身（要在断言里看落盘内容的用例用它——同一份状态）。
pub(crate) fn test_history_of(
    store: Arc<InMemoryHistory>,
) -> Arc<dyn crate::capabilities::session::api::History + Send + Sync> {
    Arc::new(crate::capabilities::session::service::SessionService::new(
        store,
    ))
}

/// 测试用的 **tools 能力**：两张表 + 三个替身端口（与生产同一条路，R12）。
/// 两个面都从这里出：`Arc<dyn Tools>`（表）与 `Arc<dyn ToolExec>`（执行）。
pub(crate) fn test_tools_svc() -> Arc<crate::capabilities::tools::service::ToolsService> {
    test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoFenceHost),
    )
}

/// 同上，但指定三个端口（断言并发/落盘/撤权的那几条用例用）。
pub(crate) fn test_tools_svc_with(
    runner: Arc<dyn crate::kernel::ports::ProcessRunner>,
    io: Arc<InMemorySysIo>,
    fence: Arc<dyn crate::kernel::ports::FenceHost + Send + Sync>,
) -> Arc<crate::capabilities::tools::service::ToolsService> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source =
        crate::capabilities::tools::detail::yaml_systools::YamlSystools::new(root.join("systools"));
    Arc::new(
        crate::capabilities::tools::service::ToolsService::new(&source, runner, io, fence)
            .expect("内置工具总表必须合法"),
    )
}

pub(crate) fn test_systools() -> crate::capabilities::tools::api::SystemTools {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate::capabilities::tools::detail::yaml_systools::YamlSystools::new(root.join("systools"))
        .load()
        .expect("内置工具总表必须合法")
}
/// 宿主探测替身：**只按给定答案回答**，不读真实环境（测试要确定性）。
/// 事实由用例显式声明；真实适配器的契约另有 T2 用例（见 docs/testing/doubles.md 端口矩阵）。
#[derive(Default)]
pub(crate) struct FixedProbe {
    pub files: Vec<std::path::PathBuf>,
    pub dirs: Vec<std::path::PathBuf>,
    pub exes: Vec<String>,
    pub hypervisor: bool,
}

impl crate::kernel::ports::HostProbe for FixedProbe {
    fn is_file(&self, path: &std::path::Path) -> bool {
        self.files.iter().any(|p| p == path)
    }
    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.dirs.iter().any(|p| p == path)
    }
    fn has_exe(&self, name: &str) -> bool {
        self.exes.iter().any(|n| n == name)
    }
    fn hypervisor_available(&self) -> bool {
        self.hypervisor
    }
}

/// 日志能力替身：什么都不做（呈现层的埋点不参与任何判定）。
pub(crate) struct NoopLogOps;
impl crate::capabilities::conductor::api::LogOps for NoopLogOps {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}
/// 测试用的提示词册能力：替身装载器 → 真实册子 → 能力面（与生产同一条路）。
pub(crate) fn test_prompt() -> std::sync::Arc<dyn crate::capabilities::prompt::api::Prompt> {
    crate::capabilities::prompt::service::load(&TestPrompts::ok()).expect("内置提示词册必须合法")
}

/// 登记处能力的测试装配：内存登记处 + 指定模型目录 + 指定通道 + 空日志。
/// 生产里这些端口由组合根注入，测试这里用替身顶。
pub(crate) fn registry_service(
    store: InMemorySettings,
    llm: Arc<dyn crate::capabilities::llm::api::Llm + Send + Sync>,
) -> Box<dyn crate::capabilities::registry::api::Registry> {
    Box::new(
        crate::capabilities::registry::service::RegistryService::new(
            Arc::new(store),
            llm,
            Arc::new(crate::kernel::ports::NoopLog),
        )
        .expect("内存登记处装配不应失败"),
    )
}

/// 测试用的 **llm 能力面**：把通道工厂 + 模型目录装进 `LlmService`（修信封用 `NoRepair`）。
/// 与生产同一条路——组合根装 service，别人只拿 `api::Llm` 面（R12）。
pub(crate) fn test_llm(
    gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync>,
    catalog: Arc<FakeCatalog>,
) -> Arc<dyn crate::capabilities::llm::api::Llm + Send + Sync> {
    Arc::new(crate::capabilities::llm::service::LlmService::new(
        gateway,
        catalog,
        Arc::new(NoRepair),
    ))
}

/// 同上，但指定信封修复器（"修信封"路径的用例用真修复器）。
pub(crate) fn test_llm_with_repair(
    repair: Arc<dyn crate::capabilities::llm::ports::EnvelopeRepair + Send + Sync>,
) -> Arc<dyn crate::capabilities::llm::api::Llm + Send + Sync> {
    Arc::new(crate::capabilities::llm::service::LlmService::new(
        Arc::new(crate::capabilities::llm::detail::fake_chat::DemoGateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        repair,
    ))
}

/// 只要一个 llm 面（不关心通道与修复）的用例用它：演示通道 + 不修。
pub(crate) fn test_llm_demo() -> Arc<dyn crate::capabilities::llm::api::Llm + Send + Sync> {
    test_llm_with_repair(Arc::new(NoRepair))
}

/// 测试用的 **workspace 能力面**：把四个端口装进 `WorkspaceService`（与生产同一条路，R12）。
/// 版本库默认用内存替身；要观察主副本/提交记录的用例用 `test_workspace_store`。
pub(crate) fn test_workspace(
    source: Arc<dyn ModuleSource + Send + Sync>,
    packages: Arc<dyn PackageSource + Send + Sync>,
    dirs: Arc<dyn Workdirs + Send + Sync>,
) -> Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync> {
    test_workspace_store(source, packages, dirs, Arc::new(InMemoryWorkStore::new()))
}

pub(crate) fn test_workspace_store(
    source: Arc<dyn ModuleSource + Send + Sync>,
    packages: Arc<dyn PackageSource + Send + Sync>,
    dirs: Arc<dyn Workdirs + Send + Sync>,
    store: Arc<dyn WorkStore + Send + Sync>,
) -> Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync> {
    Arc::new(
        crate::capabilities::workspace::service::WorkspaceService::new(
            source, packages, dirs, store,
        ),
    )
}

/// 登记一个 agent（测试装配用）：**校验用的模块清单由调用方取一份**交给登记处——
/// 清单归 workspace，登记处只认事实（见 ARCHITECTURE.md §九.3）。
pub(crate) fn agent_upsert(
    core: &mut Conductor,
    name: &str,
    modules: &[&str],
    model: &str,
    note: &str,
) -> Result<(), String> {
    let roster = core.scan();
    let modules: Vec<String> = modules.iter().map(|m| m.to_string()).collect();
    core.registry_mut()
        .agent_upsert(name, &modules, model, note, &roster)
}

/// 目的：注入指定常驻服务面与隐秘字段面的装配（动作面、会话回收、env 注入的端到端断言用）。
pub(crate) fn core_with_services(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    runner: Arc<dyn crate::kernel::ports::ProcessRunner>,
    residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
) -> Conductor {
    let gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        test_workspace(
            Arc::new(VecSource(modules)),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(
            runner,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        residents,
        secrets,
    )
}

pub(crate) fn core_with(modules: Vec<Module>, gateway: ScriptGateway) -> Conductor {
    core_with_runner(modules, gateway, Arc::new(SilentRunner))
}

/// 注入指定内存工作区的装配（断言 @ 文件清单取自会话 meta.agents）。
pub(crate) fn core_with_workspace(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    ws: Arc<InMemoryWorkspace>,
) -> Conductor {
    let gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        test_workspace(
            Arc::new(VecSource(modules)),
            Arc::new(InMemoryPackages::empty()),
            ws,
        ),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
}

/// 注入指定内存文件系统的装配（断言内置文件工具真正落盘）。
pub(crate) fn core_with_io(
    modules: Vec<Module>,
    gateway: ScriptGateway,
    io: Arc<InMemorySysIo>,
) -> Conductor {
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
    runner: Arc<impl ProcessRunner + 'static>,
) -> Conductor {
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
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
) -> Conductor {
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
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Conductor {
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
    runner: Arc<impl ProcessRunner + 'static>,
    catalog: Arc<FakeCatalog>,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
    packages: Arc<InMemoryPackages>,
) -> Conductor {
    let gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(Arc::clone(&gateway), catalog);
    Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history_of(history),
        test_workspace(
            Arc::new(VecSource(modules)),
            packages,
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(runner, io, Arc::new(NoFenceHost)),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
}

/// 用**指定登记处**装配（断言"全局设置是流式的上限、预算全局通用"这类判据）。
pub(crate) fn core_with_settings(store: InMemorySettings) -> Conductor {
    let gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gw(BTreeMap::new(), Vec::new()));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        registry_service(store, Arc::clone(&llm)),
        test_history(),
        test_workspace(
            Arc::new(VecSource(Vec::new())),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
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
) -> Conductor {
    let gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        test_workspace(
            Arc::new(VecSource(modules)),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
}

/// 指定任意网关的装配（入站契约测试用：需要自定义时序的通道）。
pub(crate) fn core_with_gateway(
    modules: Vec<Module>,
    gateway: impl ChatGateway + Send + Sync + 'static,
) -> Conductor {
    let gateway: Arc<dyn crate::capabilities::llm::ports::ChatGateway + Send + Sync> =
        Arc::new(gateway);
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history(),
        test_workspace(
            Arc::new(VecSource(modules)),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    )
}

// ---------- 核心代理（core_proxy）工具的宿主替身 ----------

use crate::capabilities::conductor::domain::proxy as dproxy;
use crate::capabilities::conductor::ports::ProxyHost;

/// 代理工具的宿主替身：只按用例给的答案回答，并把每次动作记进日志
/// （用户可见的“未授权 = 根本没碰宿主”就靠这份日志钉住）。
pub(crate) struct FakeProxyHost {
    log: Mutex<Vec<String>>,
    facts: dproxy::Catalog,
    fail_create: Mutex<Option<String>>,
    fail_send: Mutex<Vec<String>>,
    fail_control: Mutex<Vec<String>>,
    created: Mutex<Vec<dproxy::NewSession>>,
    relayed: Mutex<Vec<dproxy::Relayed>>,
    events: Mutex<Vec<String>>,
}

impl FakeProxyHost {
    pub(crate) fn new() -> FakeProxyHost {
        FakeProxyHost {
            log: Mutex::new(Vec::new()),
            facts: FakeProxyHost::facts(),
            fail_create: Mutex::new(None),
            fail_send: Mutex::new(Vec::new()),
            fail_control: Mutex::new(Vec::new()),
            created: Mutex::new(Vec::new()),
            relayed: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    fn facts() -> dproxy::Catalog {
        dproxy::Catalog {
            agents: vec![dproxy::AgentFact {
                name: "a".to_string(),
                modules: vec!["m1".to_string()],
                model: Some("gpt".to_string()),
                note: "已存".to_string(),
            }],
            modules: vec![
                dproxy::ModuleFact {
                    id: "m1".to_string(),
                    tools: vec!["t".to_string()],
                },
                dproxy::ModuleFact {
                    id: "m2".to_string(),
                    tools: Vec::new(),
                },
            ],
            models: vec![
                dproxy::ModelFact {
                    id: "gpt".to_string(),
                    name: "GPT".to_string(),
                    tools: "native".to_string(),
                },
                // 第二个模型：让“模型越出授权范围”能被单独钉住（否则会先被“无此模型”挡下）。
                dproxy::ModelFact {
                    id: "other".to_string(),
                    name: "Other".to_string(),
                    tools: "envelope".to_string(),
                },
            ],
        }
    }

    /// 记下的每一次宿主动作（断言“未授权/越界时什么都没发生”）。
    pub(crate) fn calls(&self) -> Vec<String> {
        self.log.lock().expect("锁").clone()
    }

    pub(crate) fn created(&self) -> Vec<dproxy::NewSession> {
        self.created.lock().expect("锁").clone()
    }

    pub(crate) fn relayed(&self) -> Vec<dproxy::Relayed> {
        self.relayed.lock().expect("锁").clone()
    }

    /// 预置一条消息（观察的计数与消息倒查都读它）。
    pub(crate) fn add_event(&self, text: &str) {
        self.events.lock().expect("锁").push(text.to_string());
    }

    pub(crate) fn fail_create(&self, why: &str) {
        *self.fail_create.lock().expect("锁") = Some(why.to_string());
    }

    pub(crate) fn clear_fail_create(&self) {
        *self.fail_create.lock().expect("锁") = None;
    }

    pub(crate) fn fail_send(&self, target: &str) {
        self.fail_send.lock().expect("锁").push(target.to_string());
    }

    pub(crate) fn fail_control(&self, action: &str) {
        self.fail_control
            .lock()
            .expect("锁")
            .push(action.to_string());
    }
}

impl ProxyHost for FakeProxyHost {
    fn catalog(&self, _scope: dproxy::CatalogScope) -> Result<dproxy::Catalog, String> {
        self.log.lock().expect("锁").push("catalog".to_string());
        Ok(self.facts.clone())
    }

    fn create_session(&self, spec: &dproxy::NewSession) -> Result<dproxy::Created, String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("create:{}", spec.request_id));
        if let Some(why) = self.fail_create.lock().expect("锁").clone() {
            return Err(why);
        }
        self.created.lock().expect("锁").push(spec.clone());
        Ok(dproxy::Created {
            session: format!("work-{}", spec.request_id),
            agents: spec.agents.iter().map(|a| a.name.clone()).collect(),
        })
    }

    fn send(&self, target: &str, msg: &dproxy::Relayed) -> Result<(), String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("send:{}:{}", target, msg.kind.as_str()));
        self.relayed.lock().expect("锁").push(msg.clone());
        if self
            .fail_send
            .lock()
            .expect("锁")
            .iter()
            .any(|t| t == target)
        {
            return Err(format!("目标 {} 不存在或已关闭", target));
        }
        Ok(())
    }

    fn observe(
        &self,
        session: &str,
        view: dproxy::ObserveView,
        since: Option<&str>,
    ) -> Result<dproxy::Snapshot, String> {
        self.log.lock().expect("锁").push(format!(
            "observe:{}:{:?}:{}",
            session,
            view,
            since.unwrap_or("")
        ));
        // 观察只回元信息：**不回消息正文**（正文走 messages 倒查）。
        let count = self.events.lock().expect("锁").len();
        Ok(dproxy::Snapshot {
            session: session.to_string(),
            state: "running".to_string(),
            pending: None,
            artifacts: None,
            message_count: Some(count),
            cursor: Some(count.to_string()),
            new_messages: None,
        })
    }

    fn messages(
        &self,
        session: &str,
        from: usize,
        count: usize,
    ) -> Result<dproxy::MessagesPage, String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("messages:{}:{}:{}", session, from, count));
        let events = self.events.lock().expect("锁").clone();
        let total = events.len();
        let mut out = Vec::new();
        let mut i = from;
        while i < total && out.len() < count {
            let idx = total - 1 - i;
            out.push(dproxy::MessageLine {
                id: idx as u64 + 1,
                speaker: "a".to_string(),
                verb: String::new(),
                kind: "msg".to_string(),
                text: events[idx].clone(),
            });
            i += 1;
        }
        let next = if i < total { Some(i) } else { None };
        Ok(dproxy::MessagesPage {
            session: session.to_string(),
            messages: out,
            next,
        })
    }

    fn control(
        &self,
        session: &str,
        action: dproxy::ControlAction,
        _reason: &str,
    ) -> Result<dproxy::ControlState, String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("control:{}:{}", session, action.as_str()));
        if self
            .fail_control
            .lock()
            .expect("锁")
            .iter()
            .any(|a| a == action.as_str())
        {
            return Err(format!(
                "会话 {} 不能 {}：当前状态不允许",
                session,
                action.as_str()
            ));
        }
        Ok(dproxy::ControlState {
            session: session.to_string(),
            action,
            state: match action {
                dproxy::ControlAction::Stop => "stopped",
                dproxy::ControlAction::Continue => "active",
                dproxy::ControlAction::Close => "closed",
            }
            .to_string(),
        })
    }
}

// ---------- 常驻服务（residents）的假适配器 ----------
/// 目的：常驻服务的假适配器——不拉起任何进程，记录调用并按固定脚本提供操作。
pub(crate) struct FakeServiceAdapter {
    pub calls: Arc<Mutex<Vec<String>>>,
    envs: Mutex<Vec<(String, String)>>,
}

impl FakeServiceAdapter {
    pub(crate) fn new() -> FakeServiceAdapter {
        FakeServiceAdapter {
            calls: Arc::new(Mutex::new(Vec::new())),
            envs: Mutex::new(Vec::new()),
        }
    }
    /// 目的：start 时收到的注入项（断言 secrets 注入用）。
    pub(crate) fn envs(&self) -> Vec<(String, String)> {
        self.envs.lock().expect("锁").clone()
    }
}

struct FakeServiceInstance {
    calls: Arc<Mutex<Vec<String>>>,
    env: Vec<(String, String)>,
}

impl crate::capabilities::residents::ports::ServiceInstance for FakeServiceInstance {
    fn call(&mut self, op: &str, args: &serde_json::Value) -> Result<String, String> {
        self.calls.lock().expect("锁").push(format!("call:{}", op));
        let env: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        Ok(format!("{} {} {}", op, args, env.join(",")))
    }
    fn stop(&mut self) {
        self.calls.lock().expect("锁").push("stop".to_string());
    }
}

impl crate::capabilities::residents::ports::ServiceAdapter for FakeServiceAdapter {
    fn id(&self) -> &str {
        "fake"
    }
    fn start(
        &self,
        spec: &crate::capabilities::residents::ports::LaunchSpec,
    ) -> Result<
        (
            Box<dyn crate::capabilities::residents::ports::ServiceInstance>,
            Vec<crate::capabilities::residents::api::Operation>,
        ),
        String,
    > {
        self.calls
            .lock()
            .expect("锁")
            .push(format!("start:{}", spec.name));
        self.envs.lock().expect("锁").extend(spec.env.clone());
        Ok((
            Box::new(FakeServiceInstance {
                calls: Arc::clone(&self.calls),
                env: spec.env.clone(),
            }),
            vec![crate::capabilities::residents::api::Operation {
                name: "echo".to_string(),
                description: "回显参数".to_string(),
                params: None,
            }],
        ))
    }
}

// ---------- 隐秘字段（secrets）的内存存储替身 ----------
/// 目的：SecretStore 的内存替身——可观察写入次数与内容。
pub(crate) struct InMemorySecretStore {
    values: Mutex<BTreeMap<String, String>>,
    saves: Mutex<usize>,
}

impl InMemorySecretStore {
    pub(crate) fn new() -> InMemorySecretStore {
        InMemorySecretStore {
            values: Mutex::new(BTreeMap::new()),
            saves: Mutex::new(0),
        }
    }
    pub(crate) fn with(values: BTreeMap<String, String>) -> InMemorySecretStore {
        InMemorySecretStore {
            values: Mutex::new(values),
            saves: Mutex::new(0),
        }
    }
    pub(crate) fn saves(&self) -> usize {
        *self.saves.lock().expect("锁")
    }
}

impl crate::capabilities::secrets::ports::SecretStore for InMemorySecretStore {
    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(self.values.lock().expect("锁").clone())
    }
    fn save(&self, values: &BTreeMap<String, String>) -> Result<(), String> {
        *self.values.lock().expect("锁") = values.clone();
        *self.saves.lock().expect("锁") += 1;
        Ok(())
    }
}

/// 目的：已构建的产品可执行文件（守门进程就是它自己）；没有就 None（跳过真实进程用例）。
pub(crate) fn built_exe() -> Option<std::path::PathBuf> {
    let me = std::env::current_exe().ok()?;
    let profile_dir = me.parent()?.parent()?;
    let name = if cfg!(windows) {
        "solomni.exe"
    } else {
        "solomni"
    };
    let p = profile_dir.join(name);
    p.is_file().then_some(p)
}

/// 目的：本机可用的 python（没有就 None，如实跳过需要解释器的用例）。
pub(crate) fn python() -> Option<&'static str> {
    for name in ["python", "python3"] {
        if let Ok(o) = std::process::Command::new(name)
            .arg("-c")
            .arg("print(1)")
            .output()
        {
            if o.status.success() {
                return Some(name);
            }
        }
    }
    None
}

/// 目的：一个带给定常驻服务操作的最小成员工具环境（模型侧工具面用例用）。
pub(crate) fn member_with_service_tools(
    module: &str,
    residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    op: crate::capabilities::session::api::ServiceOp,
) -> crate::capabilities::session::api::MemberTools {
    let sb = test_sandbox(module, &[]);
    let mut modules = BTreeMap::new();
    modules.insert(
        module.to_string(),
        crate::capabilities::session::domain::tools::ModuleTools {
            root: abs(&["mods", module]),
            commands: BTreeMap::new(),
            books: BTreeMap::new(),
            parallel: std::collections::BTreeSet::new(),
            services: [(op.tool.clone(), op)].into_iter().collect(),
        },
    );
    crate::capabilities::session::api::MemberTools {
        mode: crate::capabilities::llm::api::ToolMode::Envelope,
        modules,
        observations: Default::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::ports::NoopLog),
        tools: test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        sandbox: sb.clone(),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::kernel::api::FenceSpec::from_sandbox(&sb, false),
        module_env: Default::default(),
        reply_seq: 0,
        line: Default::default(),
        allowed: crate::capabilities::tools::api::names(),
        role: "solo".to_string(),
        with_modules: true,
        notes: Default::default(),
        handlers: Vec::new(),
        residents,
    }
}

// ---------- 常驻服务（MCP）的脚本化长驻会话替身 ----------
/// 目的：脚本化的长驻会话替身——按脚本逐行应答，并记录收到的每一行与关闭调用。
pub(crate) struct ScriptedHost {
    pub sent: Arc<Mutex<Vec<String>>>,
    replies: Mutex<Vec<String>>,
}

impl ScriptedHost {
    pub(crate) fn new(replies: Vec<&str>) -> ScriptedHost {
        ScriptedHost {
            sent: Arc::new(Mutex::new(Vec::new())),
            replies: Mutex::new(replies.into_iter().map(str::to_string).collect()),
        }
    }
}

struct ScriptedSession {
    sent: Arc<Mutex<Vec<String>>>,
    replies: Mutex<std::collections::VecDeque<String>>,
}

impl crate::kernel::ports::Session for ScriptedSession {
    fn send(&mut self, line: &str) -> Result<(), String> {
        self.sent.lock().expect("锁").push(line.to_string());
        Ok(())
    }
    fn recv(&mut self) -> Result<String, String> {
        self.replies
            .lock()
            .expect("锁")
            .pop_front()
            .ok_or_else(|| "脚本没有更多应答".to_string())
    }
    fn kill(&mut self) {
        self.sent.lock().expect("锁").push("<kill>".to_string());
    }
}

impl crate::kernel::ports::SessionHost for ScriptedHost {
    fn open(
        &self,
        _spec: &crate::kernel::ports::SessionSpec,
    ) -> Result<Box<dyn crate::kernel::ports::Session>, String> {
        let replies: std::collections::VecDeque<String> =
            self.replies.lock().expect("锁").clone().into();
        Ok(Box::new(ScriptedSession {
            sent: Arc::clone(&self.sent),
            replies: Mutex::new(replies),
        }))
    }
}
