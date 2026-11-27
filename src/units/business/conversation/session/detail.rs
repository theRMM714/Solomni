//! 目的：本单元自己的适配器——会话历史的落盘机制锁在这里。
//! 管：`fs_history`（会话历史的落盘实现）。
//! 不管：领域逻辑（在 `domain/`）、端口定义（在 `ports.rs`）、用例编排（在 `service.rs`）；声明装载（在 `session_model/detail`）。
//! 联动：只有入口层的组合根构造它；别的单元看不到这个模块。

pub mod fs_history;
