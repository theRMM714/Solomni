//! 目的：便携 PATH 的那一段：把 <root>/bin 追加进用户 PATH，并支持摘除（unset）与按备份还原（undo）。
//! 管：状态事实、备份落点、命令入口；平台实现分到 `path/windows.rs` / `path/other.rs`。
//! 不管：注册表怎么读写（kernel 的 UserPath 端口）、shim 内容（shim.rs）。
//! 联动：entry::setup::run 分派；测试用替身端口，不碰真实 HKCU。

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other as platform;

#[cfg(any(windows, test))]
mod rules;
#[cfg(any(windows, test))]
pub(crate) use rules::*;

/// 目的：备份文件名（放产品私有区 .home；记为「我们第一次改之前」的原串与原类型）。
pub(crate) const BACKUP_FILE: &str = "setup-path.json";

/// 目的：PATH 与 shim 的可观察状态（--setup=status 的 path 段）。
#[derive(serde::Serialize)]
pub(crate) struct PathInfo {
    pub(crate) supported: bool,
    pub(crate) bin: String,
    pub(crate) has_bin: bool,
    pub(crate) shim: String,
    pub(crate) shim_present: bool,
    pub(crate) shim_current: bool,
    pub(crate) backup_present: bool,
}

/// 目的：<root>/bin 在 PATH 里的写法（不带尾斜杠）。
pub(crate) fn bin_text(root: &Path) -> String {
    root.join("bin").display().to_string()
}

/// 目的：备份文件路径（备份只在「真的要改 PATH」时写）。
pub(crate) fn backup_path(home: &Path) -> PathBuf {
    home.join(BACKUP_FILE)
}

pub(crate) use platform::{command, info, undo_command, unset_command};
#[cfg(all(windows, test))]
pub(crate) use windows::do_path;
