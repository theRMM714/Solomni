//! 目的：turn 的推进与生成驱动分区声明。
//! 管：`facts`（沙箱 / 权限 / 工具环境事实 / 上下文预算 / 单 agent 会话装配）、`admit`（会话准入）、
//!   `rebuild`（会话重建与代理会话装配）、`turn`（单 agent 回合与放取）、`flow`（协作链与裁决动作）、
//!   `history`（事件落盘与运行态口径）、`model`（调用参数与思考片段）、`bus`（事件台）、`generation`（各条生成线程）。
//! 不管：在世表与运行态、裁决队、流程契约（在 `turn_state`）；命令队列与事件台订阅出口、动作分发与动作表（在宿主）。
//! 联动：由本能力的 `api` 重导出；宿主实现 `turn::api::TurnHost` 后转调。

pub mod admit;
pub mod bus;
pub mod facts;
pub mod flow;
pub mod generation;
pub mod history;
pub mod model;
pub mod rebuild;
pub mod turn;
