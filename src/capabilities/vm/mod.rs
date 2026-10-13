//! 目的：vm（虚拟机档）单元——运行包库、执行档位与计划、就绪检测与诊断，以及 guest 本体与后端。
//! 管：运行包库、档位诊断与计划、承载判定。
//! 不管：会话 / 工作区 / 实际工具执行（分别归 session / workspace / tools）。
//! 联动：边界见 ARCHITECTURE.md §九；契约见 RUNTIME_SPEC.md 与 docs/vm/README.md。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
