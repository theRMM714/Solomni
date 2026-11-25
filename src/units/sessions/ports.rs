//! 目的：会话盘的机制端口（R12：只由 `service.rs` 持有）。
//! 管：目录存在性的判定原语。
//! 不管：会话根的拼法（在 service）；会话目录内部的布局（在消费方 detail）。
//! 联动：真实实现 `detail::FsSessionDirs` 由组合根构造。

use std::path::Path;

/// 目的：会话目录的事实来源。
pub trait SessionDirs: Send + Sync {
    /// 目的：这个目录存不存在（符号链接按目录本身判，不跟随要求由调用方决定）。
    fn is_dir(&self, dir: &Path) -> bool;
}
