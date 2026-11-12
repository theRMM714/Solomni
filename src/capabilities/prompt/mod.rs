//! 目的：prompt 能力——册子的内存形态、{{key}} 渲染与 `@` 引用改写。
//! 管：模块根；声明 api / detail / domain / ports / service 的边界。
//! 不管：文件加载机制（在 detail）；工具与角色表（在 tools 能力）；调用方装配（在组合根）。
//! 联动：对外只有 api.rs；domain 与 ports 是本能力内部（门禁会拦越界引用）。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
