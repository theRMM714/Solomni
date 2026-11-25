//! 目的：会话的纯逻辑——裁决队列、流程调度、复合流程与席位自洽断言。
//! 管：`decisions`（提问队列）/ `flow`（流程面）/ `flows`（按 id 造流程）/ `seat_check`（席位与工具面的自洽断言）/ `sequence`（复合流程）五个纯逻辑子模块。
//! 不管：IO 与落盘（历史文件在 `detail/`）；端口的定义（在 `ports.rs`）；会话运行时（会话对象、工具面、回档算术已下沉 `agentloop`）；会话身份与声明（已下沉 `session_model`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外重导出会话事实。

pub mod decisions;
pub mod flow;
pub mod flows;
pub mod seat_check;
pub mod sequence;
