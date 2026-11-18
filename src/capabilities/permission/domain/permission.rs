//! 目的：权限的纯规则——reach × effect × decision、四档 profile、共享区路径范围与子会话收窄。
//! 管：把权限规则按行为缝拆成 `base` / `profile` / `scope` / `layer` / `set` / `path` 六个子模块，并在这里统一重导出。
//! 不管：不认会话、不碰文件系统、不持端口、不持有状态；落盘在登记处与会话 meta，生效态每次现算。
//! 联动：`workspace` 用 `PermissionSet` 做落点判定，`tools` 用 `Reach` / `Effect` 生成围栏与工具面（docs/permission/README.md）。

mod base;
mod layer;
mod path;
mod profile;
mod scope;
mod set;

pub use base::*;
pub use layer::*;
pub use path::*;
pub use profile::*;
pub use scope::*;
pub use set::*;
