//! 目的：`registry_model` 的实现——按概念分文件。
//! 管：`agents`（agent 存档形态与名字规则）。
//! 不管：读模型之外的任何用例（在 catalog）。
//! 联动：由本单元的 `api` 重导出。

pub mod agents;
