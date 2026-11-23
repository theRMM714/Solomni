//! 目的：会话的纯逻辑——会话状态与簿记、转录行与事件词汇、历史视图。
//! 管：`assemble` / `decisions` / `events` / `flow` / `flows` / `history` / `mode` / `persist` / `profile` / `rewind` / `sequence` / `session` / `tools` 十三个纯逻辑子模块。
//! 不管：IO 与落盘（历史文件在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外重导出会话事实。

pub mod assemble;
pub mod decisions;
pub mod events;
pub mod flow;
pub mod flows;
pub mod history;
pub mod mode;
pub mod persist;
pub mod profile;
pub mod rewind;
pub mod sequence;
pub mod session;
pub mod tools;
