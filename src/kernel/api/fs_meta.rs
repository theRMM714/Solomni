//! 目的：本地文件属性的对外面——可执行位、链接、目录同步与足迹。
//! 管：`make_executable` / `make_executable_checked` / `ensure_executable` / `write_executable` / `restrict_private_file` / `symlink` / `is_link` / `sync_dir` / `file_footprint`。
//! 不管：这些动作的平台差异（在 `detail/fs_meta` 的平台子模块）。
//! 联动：能力侧与装配层经本面调用；内核内部直接取 `detail::fs_meta`。

pub use crate::kernel::detail::fs_meta::{
    ensure_executable, file_footprint, is_link, make_executable, make_executable_checked,
    restrict_private_file, symlink, sync_dir, write_executable,
};
