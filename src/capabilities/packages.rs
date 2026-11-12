//! 目的：packages（运行包）单元——包契约、包库、目标选择与执行计划。
//! 管：package.yaml 契约、包库扫描、选包诊断与计划。
//! 不管：guest 生命周期与挂载（vm）、实际工具进程（tools）、会话装配（conductor）。
//! 联动：边界见 ARCHITECTURE.md §九；契约见 RUNTIME_SPEC.md 与 docs/packages/README.md。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
