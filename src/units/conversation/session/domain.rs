//! 目的：会话的纯逻辑——席位与工具面的自洽断言（其余机制已下沉）。
//! 管：`seat_check`（席位与工具面的自洽断言）。
//! 不管：IO 与落盘（历史文件在 `detail/`）；端口的定义（在 `ports.rs`）；会话运行时（已下沉 `agentloop`）；
//!   会话身份与声明（已下沉 `session_model`）；裁决队与流程面（已下沉 `turn_state`）。
//! 联动：由本单元的 `service.rs` 消费；`api.rs` 对外重导出会话事实。

pub mod seat_check;
