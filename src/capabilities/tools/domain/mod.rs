//! 目的：工具的纯逻辑——放行与寻址、参数契约、补丁解析、角色表、围栏策略。
//! 管：`fence` / `module_tools` / `patch` / `roles` / `schema` / `systool` 六个子模块。
//! 不管：起进程、装围栏、读写系统工具册这些机制（在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service/` 消费。

pub mod fence;
pub mod module_tools;
pub mod patch;
pub mod roles;
pub mod schema;
pub mod systool;
