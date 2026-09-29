//! 机制型业务：**无领域语义、无领域状态**的机制。所有业务向下依赖它，它不依赖任何人。
//!
//! 形状与别的业务一致：`api`（共享事实与纯机制）/ `ports`（机制端口，全项目共享）/ `domain`（纯机制逻辑）/ `detail`（实现）。
//! 判据（见 ARCHITECTURE.md §九.3「机制型」）：
//! 这个东西会不会需要知道"什么是回合、什么是回复、什么是工具执行"？会 = 业务，不会 = 机制。
//! 依赖方向由 T0 结构审查的**依赖方向门禁**机器判定（tests/dependency-baseline.json）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
