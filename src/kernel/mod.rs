//! 目的：机制型业务——无领域语义、无领域状态的机制；所有业务向下依赖它，它不依赖任何人。
//! 管：四个子模块的形状——`api`（共享事实与纯机制）/ `ports`（全项目共享的机制端口）/ `domain`（纯机制逻辑）/ `detail`（实现）。
//! 不管：任何业务语义——判据是"它会不会需要知道回合、回复、工具执行"：会 = 业务，不会 = 机制（见 ARCHITECTURE.md 的「三分法」）。
//! 联动：依赖方向由 T0 的依赖方向门禁机器判定（`tests/dependency-baseline.json`）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
