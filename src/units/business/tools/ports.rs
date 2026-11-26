//! 目的：工具单元的出站端口——文件读写与三张表的加载（机制在适配层）。
//! 管：`SystoolsSource` / `SysIo` 与它们的数据形态（`FileRead` / `DirEntry`）。
//! 不管：外部进程执行与围栏授权释放——那是 kernel 共享的 `ProcessRunner` / `FenceHost`。
//! 联动：由 `service.rs` 持有（R12）；kernel 端口见 `src/kernel/ports.rs`。

use crate::units::business::tools::api::LineRange;
use crate::units::business::tools::domain::table::SystemTools;
use std::path::{Path, PathBuf};

/// 目的：工具总表与名字表的加载端口：读 `systools/tools.yaml` + `systools/names.yaml`（机制在适配层）。
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
    /// 目的：**整份**原始字节的 SHA-256（不是截断前缀）；读取失败不算。
    pub hash: String,
}

/// 目的：一次按行窗口的读取：只取请求区间，但按整份文件统计行数与版本。
/// 约束：`text` 里的行与 `str::lines()` 同口径（不含行尾换行，末尾 `\r` 去掉）；
///   `window_cut` 表示还没取到 `limit` 就被单次字节上限截住，`line_cut` 表示某行本身太长只取了前缀。
#[derive(Debug)]
pub struct FilePage {
    /// 目的：选中的行，按 `\n` 连接。
    pub text: String,
    /// 目的：文件原始字节数（不是选中行的大小）。
    pub bytes: usize,
    /// 目的：选中的行里含非法 UTF-8 字节（按替换字符呈现）。
    pub lossy: bool,
    /// 目的：选中行的字节数到了单次上限，窗口提前结束。
    pub window_cut: bool,
    /// 目的：选中的某一行本身超过单次上限，只取了前缀。
    pub line_cut: bool,
    /// 目的：**整份**原始字节的 SHA-256。
    pub hash: String,
    /// 目的：整份文件的行数（与 `read` 的 `lines()` 同口径）。
    pub total_lines: u32,
}

/// 目的：目录里的一项（列目录用；大小只对文件有意义，目录恒为 0）。
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
}

/// 目的：一次写入对磁盘现状的期望——拿它做写时 CAS，检查与写盘在同一把锁里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// 目的：这个路径必须还不存在（新建文件）。
    Absent,
    /// 目的：这个路径的整份内容必须仍是这个 SHA-256。
    Version(String),
}

/// 目的：一次写入要做的变换；机制层按当前内容重新执行，不直接信任调用方算好的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    /// 目的：整份替换成这段文本。
    Replace(String),
    /// 目的：按字面替换；`all=false` 时要求恰好一处命中。
    Edit { old: String, new: String, all: bool },
}

/// 目的：事务里的一项写入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOp {
    /// 目的：要写的真实路径。
    pub path: PathBuf,
    /// 目的：写之前的磁盘版本期望。
    pub expect: Expect,
    /// 目的：写什么。
    pub mutation: Mutation,
}

/// 目的：一次成功写入的回执：新版本、逻辑行数、改动行区间与替换处数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReceipt {
    /// 目的：写后整份内容的 SHA-256。
    pub version: String,
    /// 目的：写后逻辑行数（与 read 的 `lines()` 同口径）。
    pub total_lines: u32,
    /// 目的：本次改动落到新版本里的行区间（1 起、闭区间、已合并）。
    pub changed: Vec<LineRange>,
    /// 目的：字面替换的命中处数；整份替换恒为 0。
    pub replaced: usize,
}

/// 目的：写盘失败的稳定形状：错误码给模型/UI 分流，文案直接回给模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFault {
    /// 目的：稳定错误码（FS_STALE_VERSION / FS_ALREADY_EXISTS 等）。
    pub code: &'static str,
    /// 目的：给模型看的中文说明。
    pub message: String,
}

impl WriteFault {
    /// 目的：按错误码与文案造一个写盘失败。
    pub fn new(code: &'static str, message: String) -> WriteFault {
        WriteFault { code, message }
    }

    /// 目的：磁盘上的版本已经不是调用方以为的那一份。
    pub fn stale(path: &Path, expected: &str, current: &str) -> WriteFault {
        WriteFault::new(
            "FS_STALE_VERSION",
            format!(
                "{} 在你读过之后又被改动过（内容与当时不同：期望 {}，现在是 {}）：请重新 read 一遍再决定怎么写，不要凭记忆覆盖。",
                path.display(),
                expected,
                current
            ),
        )
    }

    /// 目的：想新建的路径已经存在（别的写者抢先建成）。
    pub fn already_exists(path: &Path) -> WriteFault {
        WriteFault::new(
            "FS_ALREADY_EXISTS",
            format!(
                "{} 在写入前已经存在（别的写者抢先建成）：这次没有覆盖它，请重新 read 后再决定。",
                path.display()
            ),
        )
    }

    /// 目的：期望改已有文件，但文件已经不在了。
    pub fn missing(path: &Path) -> WriteFault {
        WriteFault::new(
            "FS_NOT_FOUND",
            format!("{} 在写入前已经不存在：这次没有写盘。", path.display()),
        )
    }

    /// 目的：按字面找不到要替换的原文。
    pub fn anchor_not_found(path: &Path) -> WriteFault {
        WriteFault::new(
            "FS_ANCHOR_NOT_FOUND",
            format!(
                "{} 里找不到 old_string（按字面匹配）；文件在读取之后可能已被改动，请重读后再改。",
                path.display()
            ),
        )
    }

    /// 目的：按字面命中多处，而这次要求唯一。
    pub fn anchor_ambiguous(path: &Path, hits: usize) -> WriteFault {
        WriteFault::new(
            "FS_ANCHOR_AMBIGUOUS",
            format!(
                "{} 里 old_string 命中 {} 处，而这次要求唯一：补足上下文，或改用整份替换。",
                path.display(),
                hits
            ),
        )
    }

    /// 目的：文件不是合法 UTF-8，不能按文本改写。
    pub fn invalid_utf8(path: &Path) -> WriteFault {
        WriteFault::new(
            "FS_INVALID_UTF8",
            format!(
                "{} 含非法 UTF-8 字节，本程序不猜编码：改写会把整份文件重写成 UTF-8，所以没有写盘。",
                path.display()
            ),
        )
    }

    /// 目的：其它机制错误。
    pub fn io(path: &Path, why: impl std::fmt::Display) -> WriteFault {
        WriteFault::new("FS_IO", format!("{} 写盘失败：{}", path.display(), why))
    }
}

impl std::fmt::Display for WriteFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

/// 目的：遍历时的过滤开关——隐藏文件与忽略文件（.gitignore / .ignore / 工作区规则）各一个开关。
///    约束：两个开关互相独立；关掉忽略不等于放行隐藏文件（与 rg 的 --no-ignore / --hidden 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkOpts {
    /// 目的：true = 连点号开头的文件/目录也遍历。
    pub hidden: bool,
    /// 目的：true = 按 .gitignore / .ignore / 工作区过滤规则剪枝。
    pub ignore: bool,
}

/// 目的：遍历里认出的一**个文件**（目录只用于剪枝，不进清单）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// 目的：相对遍历根的写法，一律 / 分隔（排序也按它）。
    pub rel: String,
    /// 目的：文件字节数（0 = 空文件或读不到大小）。
    pub bytes: u64,
}

/// 目的：单文件流式扫描的结果：这个文件是不是二进制、共几行、有没有编码/长行损失。
///    约束：二进制文件一行都不回调；长行被截到 MAX_SCAN_LINE_BYTES 后计进 long_lines。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// 目的：前 8 KiB 里出现 NUL = 二进制，这次没有扫任何一行。
    pub binary: bool,
    /// 目的：扫到的行数（含末行没有换行符的情况）。
    pub lines: u64,
    /// 目的：存在非法 UTF-8 字节，交给回调的行是按替换字符呈现的近似内容。
    pub lossy: bool,
    /// 目的：超长行（被截到上限）的条数。
    pub long_lines: u64,
}

/// 目的：扫描单行时最多读多少字节；超出的部分丢掉并计入 `long_lines`。
pub const MAX_SCAN_LINE_BYTES: usize = 64 * 1024;

/// 目的：内置文件工具的读写端口：机制在适配层，放行/寻址/越界在本单元。
///   读严格按 UTF-8 解码，非法字节如实标注；写一律 UTF-8；列目录只报名字/类型/大小。
pub trait SysIo: Send + Sync {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String>;
    /// 目的：按行窗口读取：只把 `offset` 起的 `limit` 行取回，同时给出整份行数与版本。
    /// 约束：`offset` 从 1 数起；实现必须流式扫完整份文件来统计行数与哈希，不能把前缀当全文。
    fn read_page(
        &self,
        path: &std::path::Path,
        offset: u32,
        limit: u32,
    ) -> Result<FilePage, String>;
    /// 目的：整份文件的 SHA-256；不存在 = Ok(None)，其它错误如实报。
    /// 约束：按原始字节流式算，不经过 UTF-8 解码；不用于判断目录。
    fn fingerprint(&self, path: &std::path::Path) -> Result<Option<String>, String>;
    /// 目的：锁内读当前版本 → 校验 `expect` → 应用 `mutation` → 原子替换。
    /// 约束：检查与写盘在同一把路径锁里（实现可跨进程）；版本不符时不写盘。
    fn mutate(
        &self,
        path: &std::path::Path,
        expect: &Expect,
        mutation: Mutation,
    ) -> Result<WriteReceipt, WriteFault>;
    /// 目的：一组跨文件写入：全部校验、全部暂存，发布失败时把已发布的按逆序回滚。
    /// 约束：同一路径只允许出现一次；空事务直接成功。
    fn transact(&self, ops: Vec<WriteOp>) -> Result<Vec<WriteReceipt>, WriteFault>;
    /// 列一个目录（按名字排序）。路径不是目录时如实报错——调用方据此把 read 的失败引导到 list。
    fn list(&self, path: &std::path::Path) -> Result<Vec<DirEntry>, String>;
    /// 校验 `path` 的真实落点仍在 `root` 之内：词法上的根内路径可能是符号链接 / junction，
    /// 真实目标在根外时一律拒绝。`root` 是 `Sandbox::root_of(place)` 给出的宿主根。
    fn within(&self, root: &std::path::Path, path: &std::path::Path) -> Result<(), String>;
    /// 校验 `path` 的**父目录**真实落点在 `root` 内：删除链接本身是允许的，
    /// 但不能因为链接指向根外就顺着它去删目标。默认实现用 `within(root, parent)`。
    fn within_parent(&self, root: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("路径没有父目录：{}", path.display()))?;
        self.within(root, parent)
    }
    /// 这个路径是不是目录（符号链接一律按非目录处理：删除时只删链接本身）。
    fn is_dir(&self, path: &std::path::Path) -> Result<bool, String>;
    /// 删除一个文件 / 链接；`recursive=true` 时删除整棵目录（不跟随其内部的链接）。
    fn delete(&self, path: &std::path::Path, recursive: bool) -> Result<(), String>;
    /// 目的：读一份小配置文本（不过哈希、不按读上限截断）：不存在 = Ok(None)。
    /// 约束：只给 `.workignore` 这类工作区级配置用；正文读取仍走 `read` / `read_page`。
    fn read_small(&self, path: &std::path::Path) -> Result<Option<String>, String>;
    /// 目的：递归列出 `root` 下的**文件**（按相对路径排序），过滤交给 `opts` 与 `rules`。
    /// 约束：不跟符号链接；`rules` 是工作区过滤规则全文（`.workignore` 语法），`opts.ignore` 为 false 时完全不用它；
    ///   权限不在这里判（调用方逐项判，拿不到读权限就不扫）。
    fn walk(
        &self,
        root: &std::path::Path,
        rules: Option<&str>,
        opts: WalkOpts,
    ) -> Result<Vec<WalkEntry>, String>;
    /// 目的：流式扫一份文本文件，逐行回调（行号从 1 数），不把整份读进内存。
    /// 约束：回调返回后继续；二进制（前 8 KiB 含 NUL）直接回头 `binary=true`，不回调任何行。
    fn scan_lines(
        &self,
        path: &std::path::Path,
        each: &mut dyn FnMut(u32, &str),
    ) -> Result<ScanSummary, String>;
}
