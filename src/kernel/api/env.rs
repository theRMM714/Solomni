//! 目的：环境机制对外面——单元只经这里读宿主事实与产品开关，不自己摸 `std::env`。
//! 管：重导出 `var` / `os` 与具名宿主事实（PATH、PATHEXT、SystemRoot、LOCALAPPDATA、可执行文件、临时目录）。
//! 不管：值的真假判断（`enabled(...)` 之类留在调用方）；写环境（启动面自己做）。
//! 联动：实现在 `src/kernel/detail/io/env.rs`；provision 的 fetch、vm 的共享开关与各单元的取证路径经它取。

pub use crate::kernel::detail::io::env::{current_exe, temp_dir, var};
