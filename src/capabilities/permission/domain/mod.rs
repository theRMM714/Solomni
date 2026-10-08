//! 目的：权限的纯规则——决定粒度、工作区路径的白名单 / 黑名单、模块写授权。
//! 管：`permission` 子模块（解析后的权限集合与逐条判定）。
//! 不管：IO 与落盘；这些权限从哪来（会话的 `meta.yaml` 与登记处）。
//! 联动：由 `src/capabilities/permission/api.rs` 对外导出，消费方是工作区、会话与协调业务。

pub mod permission;
