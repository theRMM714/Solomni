//! 目的：会话历史生命周期机制的装配。
//! 管：`history`（列表 / 打开）与 `delete`（删除收尾段）。
//! 不管：纯派生（`domain/`）；会话状态与删除前的内存动作（`session`、`conductor`）。
//! 联动：端口来自 `session::api`；只经 `domain/history.rs` 取派生，不碰其它能力内部。

pub mod delete;
pub mod history;
