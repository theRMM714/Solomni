//! 目的：回合状态 turn——在世会话表、运行态与创建占位、裁决队、每会话的 shell 活进程槽。
//! 管：能力分区声明；状态与访问器在 `service/state.rs`，入站面在 `api.rs`。
//! 不管：会话本体（`session` / `collab`）、生成驱动与落盘（仍在 `conductor`）。
//! 联动：`conductor` 经 `api` 读写本单元状态；裁决通道来自 `session::api`。

pub mod api;
pub mod service;
