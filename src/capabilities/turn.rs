//! 目的：回合 turn——会话本体（在世表、运行态与创建占位、裁决队、shell 活进程槽）与回合推进。
//! 管：能力分区声明；会话本体在 `service/session.rs`，取事实与单 agent 整场装配在 `service/facts.rs`，入站面在 `api.rs`。
//!   会话装配（参数对象与整场入口）归 `session`（第 4 层），turn 只负责取事实、算 `env` 后交它。
//! 不管：生成驱动、落盘与命令队列（在 `runtime`）。
//! 联动：`runtime` 经 `api` 读写本单元状态；裁决通道来自 `session::api`。

pub mod api;
pub mod service;
