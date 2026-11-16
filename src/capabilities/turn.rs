//! 目的：回合 turn——会话本体（在世表、运行态与创建占位、裁决队、shell 活进程槽）与一席的装配。
//! 管：能力分区声明；会话本体与装配在 `service/session.rs` / `service/assemble.rs`，纯派生在 `domain/assemble.rs`，入站面在 `api.rs`。
//! 不管：生成驱动、落盘与命令队列（在 `conductor`）。
//! 联动：`conductor` 经 `api` 读写本单元状态；裁决通道来自 `session::api`。

pub mod api;
pub mod domain;
pub mod service;
