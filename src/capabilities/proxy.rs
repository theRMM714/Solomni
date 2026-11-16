//! 目的：proxy——核心代理工具的能力单元（值类型、纯规则、执行面与宿主契约）。
//! 管：六代理工具的入参解析、授权判据、幂等账与回执形状；代理会话那一回合挂的执行者。
//! 不管：真实会话动作（宿主实现负责）；工具名单与角色面（`systools/`）；会话机制（`session` / `turn`）。
//! 联动：conductor 只经 `api` 取用；宿主契约 `ProxyHost` 由上层实现（见 conductor 的 `ProxyBridge`）。

pub mod api;
pub mod domain;
pub(crate) mod service;
