//! 目的：权限的纯规则——只有网络两档，其余文件面由围栏固定推导。
//! 管：`base`（生效态）与 `profile`（档位与声明层）两个子模块，并在这里统一重导出。
//! 不管：不认会话、不碰文件系统、不持端口；共享区的路径归属归 workspace（`domain/scope.rs`）。
//! 联动：`workspace` 用 `PermissionSet` 与 `SharedScope` 装配 `Sandbox`，`tools` 用档位派生围栏（docs/permission/README.md）。

mod base;
mod profile;

pub use base::*;
pub use profile::*;
