//! 目的：协作的纯逻辑——协作状态派生。
//! 管：`collab_state` 子模块。
//! 不管：IO、端口与驱动（本单元的 `service/` 持它们）。
//! 联动：由本单元的 `service/` 消费。

pub mod collab_state;
pub mod events;
pub mod gates;
