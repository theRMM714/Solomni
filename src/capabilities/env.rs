//! 目的：环境事实 env——权限面 / 沙箱清单 / 围栏 / 进程环境 / 工具链根 / guest 租约的唯一派生者。
//! 管：能力分区声明；纯派生在 `domain`，入站面在 `api`。
//! 不管：取事实（`conductor` 的 history / workspace / registry）、落盘、启停 guest 与工具进程。
//! 联动：`conductor` 取好事实后经 `api` 派生；`session` 与 `tools` 只消费传下去的事实。

pub mod api;
pub mod domain;
