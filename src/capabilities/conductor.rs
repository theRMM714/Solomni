//! 目的：协调业务 conductor——跨参与方的状态与编排。
//! 管：会话在世表、命令队列与运行态；跨能力用例编排（建/开/停会话、跨会话回档 rewind、审查关卡推进、核心操作）；api 里的 Ops / 事件台 / ConductorHandle（队列代理）。
//! 不管：各能力内部实现与别人的端口（只经各能力 api 编排，不持任何别人的端口，R12；各能力也不反向调它，见 ARCHITECTURE.md §一）。
//! 联动：核心代理工具的端口 `ProxyHost` 归 `proxy` 能力（本能力只实现它，见 service/proxy.rs 的队列桥），装配在 main 组合根。

pub mod api;
pub mod service;
