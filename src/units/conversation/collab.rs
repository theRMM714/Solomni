//! 目的：协作能力（协调型）——讨论 → 整理 → 审查关卡 → 任务链驱动 → 验收。
//! 管：三个子模块 api / domain / service 的归属与能力边界。
//! 不管：IO 与驱动（在 service/，一律经对方能力的声明面 session / llm / tools / workspace / registry / prompt）；任务链的数据与图算法（在 kernel/chain.rs）。
//! 联动：呈现层与其它能力经 api 使用；任务链形状由 src/kernel/chain.rs 提供、被三个能力共享。

pub mod api;
pub mod domain;
pub mod service;
