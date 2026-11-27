//! 目的：沙箱单元——一次会话的执行环境（本机、虚拟机、以后的容器）的声明、事实与策略注册。
//! 管：`SandboxDecl`（会话只写策略 id 与参数）、`Side` / `SandboxFacts`（策略回答的事实）、策略端口与注册表。
//! 不管：策略的实现（本机在 `tools`、虚拟机在 `vm`）；命令怎么发（`agentloop` 的通道）；落盘（会话 meta 的读写方）。
//! 联动：组合根把各实现登记进 `Sandboxes`；`session_model` 与 `work` / `turn` 经 `api` 取事实。

pub mod api;
pub mod domain;
pub mod service;
