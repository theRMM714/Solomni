//! 目的：回合状态与裁决机制——会话在世表、运行态、裁决队、流程插件契约与提问桥，一处持有。
//! 管：`api` 与 `domain`：在世表 / 运行态 / 创建占位 / 裁决队 / 每会话的 shell 活进程槽 / 流程插件契约与工厂 / 复合流程 /
//!   落盘与队列两个端口面（`History` / `HistoryOps`）。
//! 不管：回合驱动与生成（在 `turn`）、流程实现（在 `conversation::collab`）、落盘实现（在 `conversation::session`）、呈现。
//! 联动：`runtime` 经 `Turn` 放取会话；`turn` 的驱动与生成线程按 `Flow` 推进任何流程；`collab` 实现 `Flow`；
//!   呈现层经 `runtime` 的队列代理实现 `HistoryOps`。
//!   只持状态与契约、不认识任何具体流程与用户可见动作，故归环 2。
pub mod api;
pub mod domain;
