//! 目的：工作区单元——席位工作面的组装（寻址根、共享区归属）与外部工作区的布局策略。
//! 管：**一席的工作面怎么拼**（`seat_benches`）与**工作区根从哪来**（产品内布局 / 外部根的策略与复检）。
//! 不管：工作面本身的形状与寻址语义（环 2 契约单元 `workbench`）；版本化（`versions`）；落盘机制（`detail/`）。
//! 联动：其它单元经 `api` 使用；`tools` / `agentloop` 只认 `workbench::api` 的形状，不认识本单元。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
