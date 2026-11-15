//! 目的：turn 的 service 分区入口——会话本体、五张状态表、装配 / 重建 / 树序 / 事实 / 计划与在世表协议的实现处。
//! 管：子模块声明：`session`（`Turn` 本体）、`state`（五张表与访问器）、`assemble`（成员工具面与单 agent 会话对象）、
//!   `protocol`（取出 / 交回 / 裁决队 / 派生）、`rebuild`（从落盘重建）、`tree`（树序与身份）、`facts`（权限面与沙箱）、
//!   `plan`（执行计划与缺包）、`proxy`（代理会话装配）。
//! 不管：对外重导出（在 api.rs）；纯规则（在 domain）；落盘、生成驱动与命令队列（仍在 `conductor`）。
//! 联动：`api` 重导出本分区的类型与函数；单元地图见 docs/turn/unit-map.md。

pub mod assemble;
pub mod facts;
pub mod plan;
pub mod protocol;
pub mod proxy;
pub mod rebuild;
pub mod session;
pub mod state;
pub mod tree;
