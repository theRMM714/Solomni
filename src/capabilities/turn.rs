//! 目的：turn 业务——会话在世表、运行态、占名表、裁决队与 shell 状态槽的唯一持有者。
//! 管：`TurnState` 的访问器：会话对象的存取与遍历、运行态标记、创建占名、裁决队句柄、shell 槽。
//! 不管：会话怎么建、回合怎么驱动、事件怎么落盘（这些用例仍在 conductor 的 service 里）；不碰 IO。
//! 联动：`conductor` 是唯一调用方，经 `api` 取 `TurnState` 与 `Session`；单元地图见 docs/turn/unit-map.md。

pub mod api;
pub mod service;
