//! 工具能力的**出站端口**：文件读写、外部进程、围栏授权的释放（机制在适配层）。

use crate::capabilities::tools::domain::fence::FenceSpec;
use crate::capabilities::tools::domain::roles::SystemTools;

/// 工具总表与角色表的加载端口：读 `systools/tools.yaml` + `systools/roles.yaml`（机制在适配层）。
pub trait SystoolsSource {
    fn load(&self) -> Result<SystemTools, String>;
}

/// 围栏授权的释放端口：会话删除时由核心请求一次，把该会话各 agent 的围栏授权撤掉。
/// 机制在适配层（confine）；本平台没有该机制时实现为空操作。调用方只提出请求，不碰任何 ACL。
pub trait FenceHost: Send + Sync {
    fn release(&self, spec: &FenceSpec) -> Result<(), String>;
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

/// 内置文件工具的读写端口：机制在适配层，放行/寻址/越界在本能力。
/// 读严格按 UTF-8 解码，非法字节如实标注；写一律 UTF-8；列目录只报名字/类型/大小。
pub trait SysIo: Send + Sync {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String>;
    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String>;
    /// 列一个目录（按名字排序）。路径不是目录时如实报错——调用方据此把 read 的失败引导到 list。
    fn list(&self, path: &std::path::Path) -> Result<Vec<DirEntry>, String>;
}

/// 一次工具执行结果：ok = 退出码成功；output 已截断（截断规则在适配层）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub ok: bool,
    pub output: String,
}

/// 工具执行端口：机制（围栏安装/进程拉起/stdin 送参/超时杀树/截断）在适配层。
/// 策略在本能力：哪个模块能调哪个工具、命令映射、可达到哪些根，由核心按 module.yaml 与沙箱派生后传入。
pub trait ToolRunner {
    fn run(&self, fence: &FenceSpec, command: &str, args_json: &str) -> ToolOutcome;
}
