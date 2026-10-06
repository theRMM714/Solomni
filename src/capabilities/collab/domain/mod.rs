//! 目的：协作的纯逻辑——协作状态派生。
//! 管：`collab_state` 子模块。
//! 不管：IO、端口与驱动（本能力的 `service/` 持它们）。
//! 联动：由本能力的 `service/` 消费。

pub mod collab_state;
