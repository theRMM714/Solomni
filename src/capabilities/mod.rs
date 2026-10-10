//! 业务能力：按业务功能垂直切分的一等公民。每个能力有自己的 `api` / `ports` / `domain` / `detail`。
//!
//! 规则（见 ARCHITECTURE.md §九）：
//! - **业务之间只经对方的 `api`**（不许碰 `domain` / `ports`）；
//! - 能力不反向依赖适配层或呈现层；
//! - 依赖方向由 T0 结构审查的**依赖方向门禁**机器判定（tests/dependency-baseline.json）。
//!
//! `conductor`（协调业务）与其它能力**平级**：它持会话在世表、命令队列与运行态，
//! 只经各能力的 `api` 编排，别人不反向调它。

pub mod collab;
pub mod conductor;
pub mod llm;
pub mod permission;
pub mod prompt;
pub mod registry;
pub mod residents;
pub mod session;
pub mod slate;
pub mod taskchain;
pub mod tools;
pub mod workspace;
