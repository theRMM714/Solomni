//! 目的：会话历史的纯派生。
//! 管：`history`（树序排列与子树收集）。
//! 不管：读盘与日志（本能力 `service/`）；删除收尾（`service/delete.rs`）。
//! 联动：由本能力的 `service/` 与 `api` 消费。

pub mod history;
