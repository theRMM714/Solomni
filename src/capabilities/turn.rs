//! 目的：turn 业务——会话在世表、运行态、占名表、裁决队与 shell 状态槽的唯一持有者；一席会话的装配构件。
//! 管：`TurnState` 的访问器（会话对象的存取与遍历、运行态标记、创建占名、裁决队句柄、shell 槽），
//!   以及成员工具面与单 agent 会话对象的装配（`domain/assemble.rs` 定规则、`service/assemble.rs` 装上端口）。
//! 不管：回合怎么驱动、事件怎么落盘（这些用例仍在 conductor 的 service 里）；不读文件、不起 guest、不碰 IO。
//! 联动：`conductor` 是唯一调用方，经 `api` 取用；单元地图见 docs/turn/unit-map.md。

pub mod api;
pub mod domain;
pub mod service;
