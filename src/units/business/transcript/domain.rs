//! 目的：转录的纯定义——只放不依赖 IO、状态与端口的事实。
//! 管：`events`（事件词汇、转录行、裁决卡与信封、上下文用量与其构造）。
//! 不管：会话状态、裁决排队（在会话单元的 decisions）；落盘与回放（在会话单元的 detail 与 service）。
//! 联动：由 src/units/business/transcript/api.rs 对外导出；归环依据见 docs/architecture/layers.md（环 2：行里嵌 llm / tools 的载荷值）。

pub mod content;
pub mod events;
