//! 目的：工作台契约单元——一席的工作面形状：读写落在哪个区、写之前要观察到什么版本。
//! 管：`FileVersion`（一份版本身份）、`Need` / `GateFault` / `GateItem` 与端口 `WriteGate`。
//! 不管：这些形状由谁实现（布局与沙箱在 `workspace`，版本化在 `versions`）；谁消费（`tools` 的内置工具、`agentloop` 的门禁实现）。
//! 联动：会话与装配只持 `Arc<dyn ...>`；实现方在组合根登记，本单元自身无状态。

pub mod api;
pub mod domain;
