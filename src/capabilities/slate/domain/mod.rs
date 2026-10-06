//! 目的：拟名单的纯规则。
//! 管：`proposal` 子模块（按模块清单与登记处事实拟出名单）。
//! 不管：IO、端口与状态（本能力没有 `detail/`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外导出。

pub mod proposal;
