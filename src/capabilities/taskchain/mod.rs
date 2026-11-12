//! 目的：任务链（纯领域业务）——协作从讨论走到交付的那张图。
//! 管：domain 里的值对象与规则（阶段派生、就绪、验收判定、rework 合法性）；api 只导出查询与派生。
//! 不管：端口、service 与 IO（没有可编排的外部动作）。
//! 联动：三个消费者都经 api——collab 驱动、session 线格式携带、呈现层渲染（docs/collab/task-chain.md、ARCHITECTURE.md §九.3）。

pub mod api;
pub mod domain;
