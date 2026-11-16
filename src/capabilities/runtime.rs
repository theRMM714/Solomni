//! 目的：第 10 层能力 runtime——**只做应用编排**：命令队列、长步骤编排、动作分发；本身没有业务状态机。
//! 管：`api` 的入站契约与呈现层词汇、`service` 的核心本体与手柄、动作目录与分发、代理入站桥。
//! 不管：各能力的用例实现（一律经它们的 `api` 面调用，不搬身体）；HTTP / JSON / 终端呈现（在 presentation）。
//! 联动：组合根 `src/main.rs` 造 `service::Core` 再 `Handle::spawn`；`entry/faces.rs` 把手柄拆成各单元自己的窄契约（层表见 layers.yaml 的 `runtime: 10`）。

pub mod api;
pub(crate) mod service;
