//! 目的：登记处的纯逻辑——四份登记的内存形态、视图与「模型 → 通道」解析。
//! 管：`agents` / `providers` 两个子模块。
//! 不管：读 yaml、密钥与探测出站（在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费；`api.rs` 对外给会话与设置界面。

pub mod agents;
pub mod providers;
