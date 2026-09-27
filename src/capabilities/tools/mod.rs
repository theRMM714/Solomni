//! 工具能力：内置工具（read / write / edit / patch / list / search / submit_report）、
//! 参数契约、角色表与工具面、围栏策略，以及它们共用的观察账本。
//!
//! **工具的实现全部锁在本能力内部**：其它能力只经 `api` 问"有哪些工具、这一席能用哪些、这次调用的结果"。
//! 机制（文件读写、拉进程、释放授权）在 `ports` 后面由适配层实现。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
