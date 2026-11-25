//! 目的：会话的纯逻辑——会话状态与簿记、裁决队列、流程调度、工具面与回档算术。
//! 管：`assemble` / `decisions` / `flow` / `flows` / `rewind` / `sequence` / `session` / `tools` 八个纯逻辑子模块，加上席位与工具总表的自洽断言（`seat_check`）。
//! 不管：IO 与落盘（历史文件在 `detail/`）；端口的定义（在 `ports.rs`）；会话身份与声明（history / profile / mechanism / persist 已下沉环 2 的 session_model）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外重导出会话事实。

pub mod assemble;
pub mod decisions;
pub mod flow;
pub mod flows;
pub mod rewind;
pub mod seat_check;
pub mod sequence;
pub mod session;
pub mod tools;
