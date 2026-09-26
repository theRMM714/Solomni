//! 业务能力：按业务功能垂直切分的一等公民。每个能力有自己的 `api` / `ports` / `domain` / `detail`。
//!
//! 规则（见 docs/architecture/refactor-plan.md §一）：
//! - **业务之间只经对方的 `api`**（不许碰 `domain` / `ports`）；
//! - 能力不反向依赖旧巨石 `core`、适配层或呈现层；
//! - 依赖方向由 T0 结构审查的**依赖方向门禁**机器判定（tests/dependency-baseline.json）。
//!
//! 迁移期：能力逐个从 `core` 里搬出来；搬空的 `core` 最终消失。

pub mod llm;
pub mod prompt;
pub mod registry;
pub mod workspace;
