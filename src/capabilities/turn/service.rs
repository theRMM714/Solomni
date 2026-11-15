//! 目的：turn 的 service 分区入口——状态容器与一席会话装配构件的实现处与子模块声明。
//! 管：`state`（五张内存表与访问器）、`assemble`（成员工具面与单 agent 会话对象）两个子模块的声明。
//! 不管：对外重导出（在 api.rs）；纯装配规则（在 domain/assemble.rs）；不碰 IO、不碰用例编排。
//! 联动：`api` 重导出本分区的类型与函数；单元地图见 docs/turn/unit-map.md。

pub mod assemble;
pub mod state;
