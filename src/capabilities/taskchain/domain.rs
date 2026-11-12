//! 目的：任务链的纯领域实现——值对象与图算法。
//! 管：`chain` 子模块（阶段划分与依赖图）。
//! 不管：IO、端口与会话（本能力只有 `api` 与纯领域，见 ARCHITECTURE.md 的「三分法」）。
//! 联动：由 `src/capabilities/taskchain/api.rs` 导出给协调业务。

pub mod chain;
