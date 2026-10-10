//! 目的：**常驻服务**（业务能力）——模块声明的常驻外部服务的统一管理（拉起、发现操作、调用、停止、按租约回收）。
//! 管：服务清单与状态、启停与开关、操作调用、租约回收；对外只有 `api` 一处。
//! 不管：协议语义（全在 `ports::ServiceAdapter` 的实现里，只由组合根构造）；进程与围栏机制（在 kernel）；
//!   模块声明的解析（在 `workspace`，本能力只消费清单事实）。
//! 联动：边界判据见 ARCHITECTURE.md §九.1（自有用例 + ≥2 调用方）；声明契约见 MODULE_SPEC.md。

pub mod api;
pub mod ports;
pub mod service;
