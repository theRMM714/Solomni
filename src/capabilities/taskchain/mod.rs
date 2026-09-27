//! **任务链**（纯领域业务）：协作从讨论走到交付的那张图。
//!
//! 有不变式、**没有端口、没有 `service`**（没有 IO 可编排）：
//! 状态是 `domain` 里的值对象（`TaskChain`），规则全在 `domain`，
//! `api` 只导出"查询与派生"（阶段、就绪、验收判定、rework 合法性）。
//! 三个消费者都经 `api`：`collab` 驱动它、`session` 的线格式携带它、呈现层渲染它
//! （见 docs/architecture/task-chain.md 与 refactor-plan §2.3「纯领域业务」）。

pub mod api;
pub mod domain;
