//! 目的：执行与作业模型的纯定义——只放不依赖 IO、状态与端口的事实。
//! 管：`exec_selection`（执行档位与执行选型）、`job`（长任务进度快照）。
//! 不管：这些值从哪读、写到哪（登记处与会话 meta 在环 2）；执行流程（vm / provision）。
//! 联动：由 src/units/pure/exec_model/api.rs 对外导出；环 1 的硬判据见 docs/architecture/layers.md。

pub mod exec_selection;
pub mod job;
