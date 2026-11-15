//! 目的：代理工具的执行面装配。
//! 管：`proxy`（执行器 `ProxyTools` 与成员侧执行者 `ProxyHandler`）。
//! 不管：纯规则（`domain/`）；宿主实现（`conductor` 的队列桥与组合根）。
//! 联动：端口在 `ports.rs`；只经 `domain/proxy.rs` 取规则，不碰其它能力内部。

pub mod proxy;
