//! 目的：kernel 的纯机制逻辑存放处——无 IO、无领域语义、无业务文案。
//! 管：命令队列（`Actor`）、生成中作业的取消表、路径的对外书写形式与产品根的布局名字、会成为目录名的名字的校验与去重后缀、按次租约与全局沙箱要求。
//! 不管：IO（在 `detail/`）；端口的定义（在 `ports`）；跨层共享的形状（在 `contracts/`）；任何领域语义。
//! 联动：由 `src/kernel/api.rs` 重导出给各单元。

pub mod actor;
pub mod fence;
pub mod jobs;
pub mod layout;
pub mod name;
pub mod path;
pub mod platform;
pub mod receipt;
