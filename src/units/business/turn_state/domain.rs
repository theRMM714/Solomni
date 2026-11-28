//! 目的：回合状态与裁决机制的内部实现（只给 `api` 用）。
//! 管：`live`（在世表与运行态）、`turn`（回合状态本体）、`decisions`（裁决队）、
//!   `flow` / `flows` / `builder`（流程插件契约、工厂、实现注册表）、`sequence`（复合流程）、
//!   `stage`（段进度的落盘事实）、`ask`（提问桥）。
//! 不管：任何具体流程的状态机、落盘实现、呈现。
//! 联动：`api` 只做转出；`turn` 与 `runtime` 经 `api` 取用。
pub mod ask;
pub mod builder;
pub mod decisions;
pub mod flow;
pub mod flows;
pub mod live;
pub mod sequence;
pub mod stage;
pub mod turn;
