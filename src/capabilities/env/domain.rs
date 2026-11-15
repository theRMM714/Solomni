//! 目的：env 的纯派生。
//! 管：`env`（权限面、沙箱清单、围栏、进程环境、工具链根与 guest 租约）。
//! 不管：取事实、落盘、启停 guest（在 `conductor` 的 service 层）。
//! 联动：由本能力的 `api` 重导出；`conductor` 经它取用。

pub mod env;
