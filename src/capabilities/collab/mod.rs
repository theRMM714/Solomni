//! 协作能力（协调型）：讨论 → 整理 → 审查关卡 → 任务链驱动 → 验收。
//!
//! 它**不自己做 IO**，也不直接碰别的能力的内部：驱动一律经对方能力的声明面
//! （`session` / `llm` / `tools` / `workspace` / `registry` / `prompt`）。
//! 任务链的**数据与图算法**在 `kernel/chain.rs`（被三个能力共享的事实类型）。

pub mod api;
pub mod domain;
