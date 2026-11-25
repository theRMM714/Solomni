//! 目的：模块的纯逻辑——`module.yaml` 契约与校验、清单。
//! 管：`module` 子模块。
//! 不管：目录遍历（在 `detail/`）；端口定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外重导出其中的共享类型。
pub mod module;
pub mod state;
