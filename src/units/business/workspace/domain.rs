//! 目的：工作区的纯逻辑——共享区路径归属与席位工作面的组装。
//! 管：`scope`（会话参数 → 生效共享区归属）、`seats`（席位工作面清单的唯一构造点）。
//! 不管：工作面与寻址的**形状与语义**（环 2 契约单元 `workbench`）；版本化规则（在 `versions`）；IO（在 `detail/`）。
//! 联动：由本单元 `service.rs` 消费；`api.rs` 对外重导出其中的共享类型。

pub mod scope;
pub mod seats;
