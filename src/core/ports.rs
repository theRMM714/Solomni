//! 核心端口：依赖倒置的边界。core 定义，adapters 实现，main 注入。
//! 端口语义：
//! - Chat：一次模型会话（收消息列表，回原始文本）。
//! - SettingsStore：登记处持久化（providers.yaml + models.yaml；核心只认内存结构）。
//! - ModelCatalog：向一条供应商通道拉取可用模型名（发现机制在适配层）。
//! - ModuleSource：模块清单来源（「清单即事实」的重扫策略由 core 执行；目录遍历机制在适配层）。
//! - ChatGateway：通道工厂（怎么建通道在适配层；「用哪条通道」由 core 解析后传入）。
//! - PromptSource：提示词册加载（文件机制在适配层；渲染纯逻辑在 core/prompt.rs）。

use crate::core::history::{HistoryView, SessionMeta};
use crate::core::module::Roster;
use crate::core::prompt::Prompts;
use crate::core::providers::{Channel, Provider, Settings};

/// 流式片段：一次调用的起点 / 正文 / 思维链。
#[derive(Debug, Clone)]
pub enum Chunk {
    /// 每次模型调用的起点：调用方据此重置本轮累积（工具多轮不会糊在一起）。
    Start,
    Text(String),
    Reasoning(String),
}

/// 一次模型会话：收消息列表，回原始文本。
/// stream = 要求供应商流式返回；on 逐片回调（非流式实现不回调）。
/// on 返回 false = 调用方要求中止，实现方必须立即停止读取并返回已产出的正文。
pub trait Chat {
    fn complete(&mut self, messages: &[Msg], stream: bool, on: &mut dyn FnMut(Chunk) -> bool) -> Raw;
}

/// 拥有所有权的会话通道（装箱端口对象；会话可跨线程移动，Web 泵线程所需）。
pub type BoxedChat = Box<dyn Chat + Send>;

/// 一条消息：role = system / user / assistant。
#[derive(Debug, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
}

impl Msg {
    pub fn system(content: impl Into<String>) -> Msg { Msg { role: "system".into(), content: content.into() } }
    pub fn user(content: impl Into<String>) -> Msg { Msg { role: "user".into(), content: content.into() } }
    pub fn assistant(content: impl Into<String>) -> Msg { Msg { role: "assistant".into(), content: content.into() } }
}

/// 一次补全的原始文本输出。
pub type Raw = String;

/// 登记处持久化端口：供应商与模型分开保存（机制/文件名在适配层）。
pub trait SettingsStore {
    fn load(&self) -> Result<Settings, String>;
    fn save(&self, settings: &Settings) -> Result<(), String>;
}

/// 供应商模型目录端口：列出一条通道当前可用的模型名（发现机制在适配层）。
pub trait ModelCatalog {
    fn list_models(&self, provider: &Provider) -> Result<Vec<String>, String>;
}

/// 模块清单来源端口。
pub trait ModuleSource {
    fn scan(&self) -> Roster;
}

/// 围栏授权的释放端口：会话删除时由核心请求一次，把该会话各 agent 的围栏授权撤掉。
/// 机制在适配层（confine）；本平台没有该机制时实现为空操作。核心只提出请求，不碰任何 ACL。
pub trait FenceHost: Send + Sync {
    fn release(&self, spec: &crate::core::fence::FenceSpec) -> Result<(), String>;
}

/// 运行包库来源端口：扫描依赖文件夹（runtimes/）里的包清单。
/// 「清单即事实」：每次调用重扫，放入即出现；清单校验、去重与冲突预检在 core（packages::Library::build），
/// 目录遍历与 yaml 解析在适配层。
pub trait PackageSource {
    fn scan(&self) -> crate::core::packages::Library;
    /// 包库所在目录（配置界面要把"把包放哪儿"如实告诉用户）。
    fn dir(&self) -> std::path::PathBuf;
}

/// 工作区端口：一次工作的 work 目录与各 agent 沙箱（目录布局机制在适配层）。
/// core 只说"哪次工作、哪些 agent"，不碰路径拼接细节。
pub trait Workspace {
    /// 准备工作区：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 界面投喂：把文件写进本工作的 work/（文件名由调用方净化）。
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在 core。
    fn roots(&self, session: &str, agents: &[String]) -> Result<crate::core::workspace::WorkRoots, String>;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn list(&self, session: &str, agents: &[String]) -> Result<crate::core::workspace::WorkFiles, String>;
}

/// 一次文件读取：文本 + 原始字节数 + 编码与截断的如实标注。
pub struct FileRead {
    pub text: String,
    /// 文件原始字节数（不是解码后的字符数）。
    pub bytes: usize,
    /// 文本含非法 UTF-8 字节，已按替换字符呈现（本程序不猜编码）。
    pub lossy: bool,
    /// 只读了开头部分（超出单次读取上限）。
    pub cut: bool,
}

/// 内置文件工具的读写端口：机制在适配层，放行/寻址/越界在 core。
/// 读严格按 UTF-8 解码，非法字节如实标注；写一律 UTF-8。
pub trait SysIo: Send + Sync {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String>;
    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String>;
}

/// 通道工厂端口：网关只负责「怎么建通道」（机制）。
/// 「用哪条模型通道」由 core 解析后传入（策略在 core）——网关不做选择。
/// channel = None 表示无可用模型：实现方必须回落演示通道并如实告知（不得静默）。
pub trait ChatGateway {
    fn member_channel(&self, channel: Option<&Channel>, module_id: &str) -> (BoxedChat, Option<String>);
    /// 核心自身通道（整理/验收/代拟/推荐）；bool = 是否演示通道（供如实告知）。
    fn core_channel(&self, channel: Option<&Channel>) -> (BoxedChat, bool);
}

/// 会话历史端口：一个会话一个目录（meta + 事件流水）。
/// 流水只追加；回档将来以 rewind 记录追加，不物理删行（会话状态 = 回放截断）。
pub trait HistoryStore {
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份唯一真相在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}

/// 提示词册加载端口。
pub trait PromptSource {
    fn load(&self) -> Result<Prompts, String>;
}

/// 一次工具执行结果：ok = 退出码成功；output 已截断（截断规则在适配层）。
pub struct ToolOutcome {
    pub ok: bool,
    pub output: String,
}

/// 工具执行端口：机制（围栏安装/进程拉起/stdin 送参/超时杀树/截断）在适配层。
/// 策略在核心：哪个模块能调哪个工具、命令映射、可达到哪些根，由核心按 module.yaml 与沙箱派生后传入。
pub trait ToolRunner {
    fn run(&self, fence: &crate::core::fence::FenceSpec, command: &str, args_json: &str) -> ToolOutcome;
}

/// 运行日志端口：关键节点（异常/降级/边界）落盘，供事后确定问题，避免过度推理。
/// core 只调用；文件/时间戳/目录机制在适配层。
pub trait Log: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

/// 测试与纯逻辑场景的无声日志（不落任何盘）。
pub struct NoopLog;
impl Log for NoopLog {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}
