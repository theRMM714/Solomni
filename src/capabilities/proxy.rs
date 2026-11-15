//! 目的：核心代理工具业务 proxy——把 `core_proxy` 六个工具的声明、授权与执行独立成一个能力。
//! 管：能力分区声明；入站面在 `api`，出站端口在 `ports`，纯规则在 `domain`，执行在 `service`。
//! 不管：会话与派发（宿主是 `conductor` 的队列桥）；工具面名单与 callers（`systools/tools.yaml`）。
//! 联动：`conductor` 实现 `ports::ProxyHost` 并经 `api` 注入执行者；装配在组合根。

pub mod api;
pub mod domain;
pub mod ports;
pub mod service;
