//! 目的：vm 的纯逻辑——档位诊断与计划派生、guest 布局与路径桥。
//! 管：exec（档位、计划、承载判定）、guest（基础根布局）与 paths（宿主↔guest 路径桥）。
//! 不管：运行包库与选包（packages）、目录遍历与落盘（detail/）、端口定义（ports.rs）。
//! 联动：被 api.rs 重导出，由 service.rs 消费。

pub mod exec;
pub mod guest;
pub mod paths;
