//! 目的：权限能力——独立于会话的一类能力，回答「谁能在本机、在哪些路径上做什么」。
//! 管：能力的模块根（api 入站面 + domain 纯规则）；权限声明对外类型与判定入口的归属。
//! 不管：落盘的声明（在登记处与会话 meta）；生效态（每次现算，不在这里持有）；具体规则（在 domain）。
//! 联动：消费方 workspace / tools / conductor 走 api.rs；入站面见 src/capabilities/permission/api.rs，规则见 domain/permission.rs。

pub mod api;
pub mod domain;
