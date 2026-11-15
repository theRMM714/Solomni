//! 目的：work 能力——把「要一份工作」翻译成会话配置：形态、名单、模型档、权限与虚拟根。
//! 管：建工作、改配、工作视图与运行报告、模型档与思考强度、guest 的启动与撤销；`Work` 是这条用例线的唯一入口。
//! 不管：会话本体与运行态（`turn`）、转录落盘（`session` 与注入的钩子）、命令队列与生成驱动（`conductor`）。
//! 联动：唯一调用方 `conductor` 经 `api` 取用；`turn` 提供会话本体，模块 / 登记处 / 宿主探测经各自 `api`。

pub mod api;
pub mod domain;
mod service;
