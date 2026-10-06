//! 目的：kernel 的纯机制逻辑与共享事实的存放处。
//! 管：无 IO、无领域语义的机制——取消表、工具放行表、路径的对外书写形式、共享事实类型。
//! 不管：IO（在 `detail/`）；端口的定义（在 `ports`）；任何领域语义。
//! 联动：由 `src/kernel/api.rs` 重导出给各能力。

pub mod approvals;
pub mod jobs;
pub mod path;
pub mod types;
