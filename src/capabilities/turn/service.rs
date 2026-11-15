//! 目的：turn 的 service 分区入口——状态容器的实现处与子模块声明。
//! 管：`state` 子模块的声明；状态容器与访问器的实现全在它里面。
//! 不管：对外重导出（在 api.rs）；不碰 IO、不碰用例编排。
//! 联动：`api` 重导出本分区 `state` 的两个类型；单元地图见 docs/turn/unit-map.md。

pub mod state;
