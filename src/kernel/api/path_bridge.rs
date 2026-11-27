//! 目的：路径桥的对外面——宿主路径与另一侧路径的互换（按方言，同方言恒等）。
//! 管：`PathBridge`（`new` / `for_session` / `to_guest` / `round_trips`）与 `PathDialect`。
//! 不管：谁在什么时候用（沙箱挂载、命令装配、提示词各自调用）、文件是否真存在。
//! 联动：`vm` 与 `agentloop` 的提示词都经本面渲染跨环境路径；内核内部直接取 `domain::path_bridge`。

pub use crate::kernel::domain::path_bridge::{PathBridge, PathDialect};
