//! 目的：提示词册的纯逻辑——册子的内存形态、`{{key}}` 渲染、`@` 引用改写。
//! 管：`prompt` / `refs` 两个子模块。
//! 不管：读提示词文件（在 `detail/`）；端口的定义（在 `ports.rs`）。
//! 联动：由本能力的 `service.rs` 消费。

pub mod prompt;
pub mod refs;
