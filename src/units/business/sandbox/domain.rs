//! 目的：沙箱单元的纯形状——声明、事实、注册项与本机策略。
//! 管：`decl`（会话声明与分类）、`facts`（策略回答的事实）、`guest`（另一侧的 OS 与架构）、`exec`（执行端口）。
//! 不管：策略注册（在 `service`）；命令通道的实现（本机在 `tools`，客机在 `vm`）。
//! 联动：由本单元的 `api` 重导出。

pub mod decl;
pub mod exec;
pub mod facts;
pub mod guest;
pub mod policy;
