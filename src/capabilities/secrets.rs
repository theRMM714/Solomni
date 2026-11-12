//! 目的：**隐秘字段**（业务能力）——模块声明的隐秘信息的值存储与按模块解析（注入项 / 脱敏）。
//! 管：`.home/` 里的值存储、模块声明的字段视图、按模块解析注入项、按已知值脱敏。
//! 不管：模块声明的解析（在 `workspace`）；实际注入到子进程（由工具/常驻服务在起进程时消费注入项）；
//!   通道密钥（归 `registry` 的 `providers.yaml`；本能力只管模块隐秘字段）。
//! 联动：声明见 MODULE_SPEC.md；落盘边界见 REGISTRY_SPEC.md §七。

pub mod api;
pub mod detail;
pub mod ports;
pub mod service;
