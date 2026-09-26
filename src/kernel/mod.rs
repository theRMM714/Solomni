//! 机制型内核：**无领域语义、无领域状态**的机制。所有业务向下依赖它，它不依赖任何人。
//!
//! 判据（见 docs/architecture/refactor-plan.md §2.2「领域词测试」）：
//! 这个东西会不会需要知道"什么是回合、什么是回复、什么是工具执行"？会 = 业务，不会 = 内核。
//! 依赖方向由 T0 结构审查的**依赖方向门禁**机器判定（tests/dependency-baseline.json）。

pub mod chain;
pub mod host;
pub mod jobs;
pub mod log;
pub mod path;
pub mod types;
