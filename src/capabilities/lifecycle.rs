//! 目的：会话历史的生命周期机制 lifecycle——树序列出 / 只读打开 / 删除收尾。
//! 管：能力分区声明；纯派生在 `domain`，读盘与删除收尾在 `service`，入站面在 `api`。
//! 不管：会话对象与会话状态（`session`）；子树运行态、常驻服务与围栏账本（`conductor`）。
//! 联动：`conductor` 经 `api` 取列表 / 打开 / 收尾；删除前的内存动作仍由它自己做。

pub mod api;
pub mod domain;
pub mod service;
