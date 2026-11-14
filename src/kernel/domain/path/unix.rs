//! 目的：`path` 的非 Windows 后端——没有扩展前缀要还原。
//! 管：原样返回路径。
//! 不管：UNC 判定与拆分（在门面，两平台共用）；对外 `/` 书写。
//! 联动：由 `src/kernel/domain/path.rs` 在非 Windows 目标选中。

use std::path::{Path, PathBuf};

/// 目的：非 Windows 没有 `\\?\` 前缀，原样返回。
pub(crate) fn native(p: &Path) -> PathBuf {
    p.to_path_buf()
}
