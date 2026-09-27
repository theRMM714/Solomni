//! **拟名单**（协调型业务）：给一句需求，让核心提一份**可直接开工**的 agent 名单。
//!
//! 边界判据（ARCHITECTURE.md §九.1）：
//! - **有自己的用例与协议**：`slate` 工具的载荷（`picks`，形状 = `registry::api::RosterPick`）
//!   与提示词段（`slate.*`）归它；用户可见后果是「推荐名单」（一次性建议）与
//!   「代拟名单」（协作会话里待用户确认）；
//! - **能指名两个调用方**：`conductor`（`ConductorOps::suggest_models`）与 `collab`（代拟），
//!   从前两处各写一遍（R13 的反例），所以它不是一个调用方的脚本、也不归任何参与方。
//!
//! **它不持端口、也不持状态**：名单是**在飞的值**，归提出方（`collab` 的待确认名单、前端的 JS）；
//! 参与方事实由调用方传入——`collab` 只持登记处**快照**（写面 `Box<dyn Registry>` 在 `conductor` 手里）。
//! 用例实现在 `service.rs`（驱动 IO），纯规则（模式收束）在 `domain/`。

pub mod api;
pub mod domain;
pub mod service;
