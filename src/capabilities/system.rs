//! 目的：系统维护与自检能力——启动相命令要的自检、回收、补齐、还原、构建与工作回收的用例面。
//! 管：能力分区声明（api / service / detail）。
//! 不管：命令语法与回执文案（在呈现层）、各能力的实现（storage / provision / builds / workspace / vm）。
//! 联动：入口的启动相命令经 presentation/cli/boot.rs 调它；契约见 docs/system/README.md。

pub mod api;
pub mod detail;
pub mod service;
