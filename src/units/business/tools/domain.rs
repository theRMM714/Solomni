//! 目的：工具的纯逻辑——放行与寻址、参数契约、补丁解析、工具总表、围栏策略。
//! 管：`budget` / `commands` / `env` / `fence` / `module_tools` / `op` / `patch` / `schema` / `shell` / `systool` / `table` 十一个子模块。
//! 不管：起进程、装围栏、读写系统工具册这些机制（在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本单元的 `service/` 消费。

pub mod budget;
pub mod commands;
pub mod env;
pub mod fence;
pub mod module_tools;
pub mod op;
pub mod patch;
pub mod schema;
pub mod shell;
pub mod systool;
pub mod table;
