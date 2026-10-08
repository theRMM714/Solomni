//! 权限：独立于会话的一类能力——回答「谁能在本机、在哪些路径上做什么」。
//! 本能力是**领域型**（同 taskchain）：纯规则、无 IO、无端口；落盘的声明在登记处与会话 meta，
//! 生效态由 `domain::permission::Permissions` 现算（不持有状态）。
//! 白名单 / 黑名单是**呈现层词汇**：解析后只剩一个正向判定（见 domain::permission 的 allowed）。

pub mod api;
pub mod domain;
