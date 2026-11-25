//! 目的：proxy 的纯形状与判据——六工具的载荷、授权判据与回执形状，不认识会话、不碰 IO。
//! 管：`proxy` 子模块。
//! 不管：宿主契约（`api.rs` 的 ProxyHost）；执行流程与幂等账（`service/proxy.rs`）。
//! 联动：由本单元的 `service/proxy.rs` 与 runtime 等调用方消费。

pub mod proxy;
