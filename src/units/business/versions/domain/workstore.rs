//! 目的：工作区版本库的领域形态——记录、路径、三方比较、行级差异、回退请求、解决判定与状态展示。
//! 管：`record` / `path` / `merge` / `diff` / `reset` / `resolve` / `status` 七个子模块，并在这里统一重导出，供 `workspace::api` 与 `service` 取用。
//! 不管：落盘与 IO（`workspace::detail::fs_workstore`）、入站面契约（`workspace::api`）。
//! 联动：`api.rs` 重导出这些类型；`src/tests` 只经入站面引用（docs/workspace/README.md）。

mod diff;
mod merge;
mod path;
mod record;
mod reset;
mod resolve;
mod status;
#[cfg(test)]
mod tests;

pub use diff::*;
pub use merge::*;
pub use path::*;
pub use record::*;
pub use reset::*;
pub use resolve::*;
pub use status::*;
