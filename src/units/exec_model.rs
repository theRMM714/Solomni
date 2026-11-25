//! 目的：执行与作业模型——执行档位、执行选型、长任务进度这三类共享词汇的唯一家。
//! 管：单元的模块根（api 入站面 + domain 纯定义）；`Tier` / `ExecSpec` / `JobState` 的归属。
//! 不管：执行怎么跑（vm / provision / maintenance 各自一段）；业务决策与面向用户的文案；任何 IO 与状态。
//! 联动：登记处、执行计划、会话 meta 与进度通道都经 api.rs 取这些类型；纯层判据见 docs/architecture/layers.md。

pub mod api;
pub mod domain;
