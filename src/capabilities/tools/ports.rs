//! 目的：工具能力的出站端口——文件读写与两张表的加载（机制在适配层）。
//! 管：`SystoolsSource` / `SysIo` 与它们的数据形态（`FileRead` / `DirEntry`）。
//! 不管：外部进程执行与围栏授权释放——那是 kernel 共享的 `ProcessRunner` / `FenceHost`。
//! 联动：由 `service/mod.rs` 持有（R12）；kernel 端口见 `src/kernel/ports.rs`。

use crate::capabilities::tools::domain::roles::SystemTools;

/// 目的：工具总表与角色表的加载端口：读 `systools/tools.yaml` + `systools/roles.yaml`（机制在适配层）。
pub trait SystoolsSource {
    fn load(&self) -> Result<SystemTools, String>;
}

/// 目的：一次文件读取：文本 + 原始字节数 + 编码与截断的如实标注。
pub struct FileRead {
    pub text: String,
    /// 目的：文件原始字节数（不是解码后的字符数）。
    pub bytes: usize,
    /// 目的：文本含非法 UTF-8 字节，已按替换字符呈现（本程序不猜编码）。
    pub lossy: bool,
    /// 目的：只读了开头部分（超出单次读取上限）。
    pub cut: bool,
}

/// 目的：目录里的一项（列目录用；大小只对文件有意义，目录恒为 0）。
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
}

/// 目的：内置文件工具的读写端口：机制在适配层，放行/寻址/越界在本能力。
///   读严格按 UTF-8 解码，非法字节如实标注；写一律 UTF-8；列目录只报名字/类型/大小。
pub trait SysIo: Send + Sync {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String>;
    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String>;
    /// 列一个目录（按名字排序）。路径不是目录时如实报错——调用方据此把 read 的失败引导到 list。
    fn list(&self, path: &std::path::Path) -> Result<Vec<DirEntry>, String>;
}
