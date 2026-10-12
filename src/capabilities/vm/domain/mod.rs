//! 目的：vm 的纯逻辑——运行包清单契约与校验、档位诊断与计划派生。
//! 管：packages（清单事实）、exec（档位、计划、承载判定）与 guest（基础根布局）。
//! 不管：目录遍历与落盘（detail/）、端口定义（ports.rs）。
//! 联动：被 api.rs 重导出，由 service.rs 消费。

pub mod exec;
pub mod guest;
pub mod packages;
