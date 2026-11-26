//! 目的：会话身份与声明的定义——按概念分文件：元信息、类型声明、机制席位、落盘策略。
//! 管：`history`（元信息与历史视图、运行态、累计用量）、`profile`（会话类型表与解析后声明）、`mechanism`（机制与席位表）、`persist`（留档策略与系统会话约定）、`tools`（工具来源端口与回合上下文）、`decisions`（裁决通道的发起面）。
//! 不管：落盘 IO 与会话对象（在会话单元）；流程调度（在 collab / turn）。
//! 联动：由本单元的 `api` 重导出；归环依据见 docs/architecture/layers.md（环 2：元信息里嵌 workspace 的共享区覆盖与 transcript 的事件词汇）。

pub mod decisions;
pub mod history;
pub mod mechanism;
pub mod persist;
pub mod profile;
pub mod tools;
