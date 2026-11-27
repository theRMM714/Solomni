//! 目的：沙箱单元的纯形状——声明、事实、注册项与本机策略。
//! 管：`decl`（会话声明与分类）、`facts`（策略回答的事实）、`host`（本机策略）。
//! 不管：策略注册（在 `service`）；命令通道。
//! 联动：由本单元的 `api` 重导出。

pub mod decl;
pub mod facts;
pub mod host;
pub mod policy;
