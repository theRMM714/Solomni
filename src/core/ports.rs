//! 核心端口：依赖倒置的边界。core 定义，adapters 实现，main 注入。
//! **已随能力搬出**：`capabilities/prompt/ports.rs`（PromptSource）、`capabilities/registry/ports.rs`（SettingsStore）、
//! `capabilities/llm/ports.rs`（Chat / ChatGateway / ModelCatalog / EnvelopeRepair）。
//! 本文件只剩尚未搬出的部分（workspace / tools / session / 宿主探测）。

use crate::core::history::{HistoryView, SessionMeta};
use crate::core::module::Roster;
use std::path::Path;

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
    fn roots(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::core::workspace::WorkRoots, String>;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn list(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::core::workspace::WorkFiles, String>;
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

/// 目录里的一项（列目录用；大小只对文件有意义，目录恒为 0）。
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
}

/// 内置文件工具的读写端口：机制在适配层，放行/寻址/越界在 core。
/// 读严格按 UTF-8 解码，非法字节如实标注；写一律 UTF-8；列目录只报名字/类型/大小。
pub trait SysIo: Send + Sync {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String>;
    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String>;
    /// 列一个目录（按名字排序）。路径不是目录时如实报错——core 据此把 read 的失败引导到 list。
    fn list(&self, path: &std::path::Path) -> Result<Vec<DirEntry>, String>;
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

/// 一次工具执行结果：ok = 退出码成功；output 已截断（截断规则在适配层）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub ok: bool,
    pub output: String,
}

/// 工具执行端口：机制（围栏安装/进程拉起/stdin 送参/超时杀树/截断）在适配层。
/// 策略在核心：哪个模块能调哪个工具、命令映射、可达到哪些根，由核心按 module.yaml 与沙箱派生后传入。
pub trait ToolRunner {
    fn run(
        &self,
        fence: &crate::core::fence::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome;
}

/// 宿主能力探测：**只问事实**——不执行任何程序、不安装、不写任何东西。
/// 机制（读环境变量、查路径存在性、按平台判定虚拟化能力）在适配层；
/// core 只按结论做判断，因此 core 里不出现 std::env 与 is_file / is_dir。
pub trait HostProbe: Send + Sync {
    /// 这个路径存在且是文件。
    fn is_file(&self, path: &Path) -> bool;
    /// 这个路径存在且是目录。
    fn is_dir(&self, path: &Path) -> bool;
    /// PATH 上有没有这个可执行文件（只查存在性，不执行它；平台扩展名由适配层处理）。
    fn has_exe(&self, name: &str) -> bool;
    /// 本机能不能起硬件虚拟化（只问事实，不起任何虚拟机）。
    fn hypervisor_available(&self) -> bool;
}
