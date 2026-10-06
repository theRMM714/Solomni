//! 目的：协调业务的纯逻辑。
//! 管：`proxy` / `work` 两个子模块（代理会话的形态与共享区工作名）。
//! 不管：端口与 IO（本能力的 `service/` 持端口）；适配器的构造（组合根）。
//! 联动：由本能力的 `service/` 消费。

pub mod proxy;
pub mod work;
