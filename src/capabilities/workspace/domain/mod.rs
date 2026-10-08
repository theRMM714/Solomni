//! 目的：工作区的纯逻辑——清单契约与校验、执行计划派生、沙箱寻址与越界判定。
//! 管：`exec` / `hash` / `module` / `packages` / `workspace` / `workstore` 六个子模块。
//! 不管：IO 与适配（扫描磁盘、读写工作区在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外重导出其中的共享类型。

pub mod exec;
pub mod hash;
pub mod module;
pub mod packages;
pub mod workspace;
pub mod workstore;
