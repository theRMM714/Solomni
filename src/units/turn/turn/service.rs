//! 目的：turn 的状态、推进与生成驱动分区声明。
//! 管：`state`（在世会话表、运行态、裁决队、shell 槽）、`session`（会话本体 `Turn`）、
//!   `facts`（沙箱 / 权限 / 工具环境事实 / 上下文预算 / 单 agent 会话装配）、`admit`（会话准入）、`rebuild`（会话重建与代理会话装配）、
//!   `turn`（单 agent 回合与放取）、`flow`（协作链与裁决动作）、`history`（事件落盘与运行态口径）、
//!   `ask_user`（工具层提问端口）、`model`（调用参数与思考片段）、`generation`（各条生成线程）。
//! 不管：命令队列与事件台订阅出口、动作分发与动作表（在宿主）。
//! 联动：由本能力的 `api` 重导出；宿主实现 `turn::api::TurnHost` 后转调。

pub mod admit;
pub mod ask_user;
pub mod bus;
pub mod facts;
pub mod flow;
pub mod generation;
pub mod history;
pub mod model;
pub mod rebuild;
pub mod session;
pub mod state;
pub mod turn;
