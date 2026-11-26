//! 目的：回合推进——会话准入与按需重建、一席的事实取用、单 agent 与协作链的推进、生成链与事件台。
//! 管：子业务分区声明；准入与重建在 `service/{admit,rebuild}.rs`，推进在 `service/{turn,flow}.rs`，生成链在 `service/generation.rs`。
//!   会话装配（参数对象与整场入口）在 `agentloop`（`domain/assembly.rs`），turn 只取事实、算 `env` 后交它。
//! 不管：生成驱动、落盘与命令队列（在 `runtime`）；在世表、运行态与裁决队（在 `turn_state`）。
//! 联动：`runtime` 经 `api` 推进回合；会话对象与装配归 `agentloop`，逐轮落盘归 `conversation` 的 `session`。

pub mod api;
pub mod service;
