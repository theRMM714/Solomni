//! 目的：文件写与事务的纯形状——期望的旧状态、失败码与失败本身。
//! 管：`Expect`（不看旧状态 / 必须不存在 / 必须是我读过的那个版本）、`Fault`（稳定码 + 中文原因）与 `FS_*` 失败码。
//! 不管：拿锁、暂存、发布与回滚（在 `detail/fs`）；准不准写（策略在 workbench 的 `BenchDriver` 与 `SharedScope`）。
//! 联动：`api::fs` 的 `write` / `append` / `stream_into` / `transact_with` 都收 `Expect`；`tools` 把 `Fault` 的码原样转进 `WriteFault`。

use std::fmt;
use std::path::Path;

/// 目的：磁盘上没能按预期改成的失败码——调用方据此回执，不解析文案。
pub const FS_IO: &str = "FS_IO";

/// 目的：同一个路径在一次事务里出现两次的失败码。
pub const FS_DUPLICATE_PATH: &str = "FS_DUPLICATE_PATH";

/// 目的：锁没拿到的失败码。
pub const FS_LOCKED: &str = "FS_LOCKED";

/// 目的：期望的旧版本与磁盘上的不一致的失败码。
pub const FS_STALE: &str = "FS_STALE";

/// 目的：期望不存在但已经有东西的失败码。
pub const FS_EXISTS: &str = "FS_EXISTS";

/// 目的：期望存在但已经没有东西的失败码。
pub const FS_MISSING: &str = "FS_MISSING";

/// 目的：路径不是绝对路径、或落点不在允许的根内的失败码。
pub const FS_OUTSIDE: &str = "FS_OUTSIDE";

/// 目的：这次改动期望磁盘上的旧状态是什么——决定写不写得成，以及发布时要不要占位。
/// 参数：`Any` 不看旧状态；`Absent` 必须还没有；`Version` 必须是读过的那个内容版本。
/// 约束：`Version` 在锁内比对整份内容的哈希，不看修改时间；比对不上就整笔不写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// 目的：不看旧状态。
    Any,
    /// 目的：目标必须还不存在。
    Absent,
    /// 目的：目标必须是这个内容版本。
    Version(String),
}

/// 目的：一次写或一笔事务没做成的原因——稳定码给回执，文案给人看。
/// 约束：`code` 只取本模块的 `FS_*` 常量；事务回滚失败时额外原因追加在 `message` 里。
#[derive(Debug, Clone)]
pub struct Fault {
    /// 目的：稳定失败码。
    pub code: &'static str,
    /// 目的：中文原因（可含路径与底层错误）。
    pub message: String,
}

impl Fault {
    /// 目的：造一个失败。
    /// 参数：`code` 稳定码；`message` 中文原因。
    pub fn new(code: &'static str, message: impl Into<String>) -> Fault {
        Fault {
            code,
            message: message.into(),
        }
    }

    /// 目的：把一次系统调用失败包成 `FS_IO`。
    /// 参数：`path` 出问题的路径；`why` 底层错误。
    pub fn io(path: &Path, why: impl fmt::Display) -> Fault {
        Fault::new(FS_IO, format!("{} 写盘失败：{}", path.display(), why))
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
