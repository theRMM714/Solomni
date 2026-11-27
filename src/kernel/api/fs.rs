//! 目的：本地文件的对外面——写、事务、删树、复制、越界校验，以及可执行位与链接这些属性。
//! 管：`write` / `write_private` / `append` / `stream_into` / `transact_with`（配合 `Expect` / `Fault` 与 `FS_*` 码）、`LockSpace` 与 `install_lock_space`、`ensure_dir` / `remove_tree` / `copy_file` / `move_file` / `within`，以及原来的 `make_executable` / `ensure_executable` / `write_executable` / `symlink` / `is_link` / `file_footprint`。
//! 不管：准不准写、写什么内容（策略在 workbench）；版本语义与读后写门禁（在 versions）；锁文件放在哪个目录（组合根注入）。
//! 联动：单元要落盘只经本面；内核内部直接取 `domain::fs` 与 `detail::fs`。

pub use crate::kernel::detail::fs::LockSpace;
pub use crate::kernel::detail::fs::{
    append, copy_file, ensure_dir, install_lock_space, move_file, remove_tree, stream_into,
    transact_with, within, write, write_private,
};
pub use crate::kernel::detail::fs_meta::{
    ensure_executable, file_footprint, is_link, make_executable, make_executable_checked, symlink,
    write_executable,
};
pub use crate::kernel::domain::fs::{Expect, FS_EXISTS, FS_IO, FS_MISSING, FS_STALE, Fault};
