//! 目的：proxy 的执行面——按声明校验参数、判授权、查幂等账，再把动作交给宿主。
//! 管：`proxy` 子模块。
//! 不管：纯判据（`domain/proxy.rs`）；宿主实现（生产在 runtime 的 `service/proxy.rs`）。
//! 联动：宿主契约见 `api.rs` 的 ProxyHost；工具名单在 `systools/tools.yaml`。

pub(crate) mod proxy;
