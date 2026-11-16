//! 目的：协调业务的纯逻辑。
//! 管：`action`（动作回包形状）子模块。
//! 不管：端口与 IO（本能力的 `service/` 持端口）；适配器的构造（组合根）。
//! 联动：由本能力的 `service/` 消费；代理会话形态与纯规则已归 `proxy` 能力（见 `src/capabilities/proxy/`）。

pub mod action;
