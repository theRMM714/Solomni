//! 目的：工具能力——内置工具（read / write / edit / patch / delete / list / search / submit_report）、
//!   参数契约、角色表与工具面、围栏策略，以及它们共用的观察账本。
//! 管：`domain` 纯逻辑、`ports` 出站端口、`service` 用例与端口持有、`detail` 适配器四块。
//! 不管：机制实现（文件读写、拉进程、释放授权）在 `detail/` 与 kernel 共享端口后面。
//! 联动：其它能力只经 `api` 提问；装配在 `main.rs`。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
