//! 目的：转录——会话流水线的线格式与呈现词汇的唯一家：事件、转录行、裁决卡、上下文用量。
//! 管：单元的模块根（api 入站面 + domain 纯定义）；落盘与窗口之间那一层形状（谁都能读，谁都不解释）。
//! 不管：会话状态与派生（在会话单元）、裁决排队与等待的机制（在裁决队列）、落盘 IO（在历史 detail）；任何 IO 与状态。
//!   归环 2 而不是环 1：行里嵌着 llm 的 `TokenUsage` 与 tools 的观察 / shell 事实，这些载荷还没下沉环 1（见 docs/transcript/README.md）。
//! 联动：会话单元产生这些值；runtime、turn、collab 与呈现层经 api.rs 取；契约见 docs/session/session-model.md。

pub mod api;
pub mod domain;
