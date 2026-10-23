//! 目的：packages 的纯逻辑——运行包清单契约与校验、选型诊断与计划派生。
//! 管：manifest（清单事实与包库）、plan（选包、问题、不可用能力）。
//! 不管：目录遍历与落盘（detail/）、端口定义（ports.rs）。
//! 联动：被 api.rs 重导出，由 service.rs 与调用方消费。

pub mod manifest;
pub mod plan;
