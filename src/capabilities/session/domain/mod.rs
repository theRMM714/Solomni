//! 目的：会话的纯逻辑——会话状态与簿记、转录行与事件词汇、历史视图。
//! 管：`events` / `history` / `rewind` / `session` / `tools` 五个子模块。
//! 不管：IO 与落盘（历史文件在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外重导出会话事实。

pub mod events;
pub mod history;
pub mod rewind;
pub mod session;
pub mod tools;
