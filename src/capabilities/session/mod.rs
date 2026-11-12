//! 目的：会话能力——一个 agent 会话的状态与簿记、转录行线格式、事件词汇与历史视图。
//! 管：对话、行 id / 回合 / 回复簿记、压缩点、回档边界；对外的面在 `api.rs`，纯逻辑在 `domain/`。
//! 不管：回合驱动（在 `engine`：驱动必须能读写会话状态，方向是 engine → session）；落盘机制（在 `ports::HistoryStore` 与 `detail/`）。
//! 联动：engine 驱动它；呈现层经 `api::HistoryOps` 读，别的能力经 `api::History` 写。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
