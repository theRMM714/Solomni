//! 目的：前端（交付机制）：各渠道一个子目录——`cli/`（终端）与 `web/`（本地网页）。
//! 管：两个渠道的模块声明与目录边界；入站面集合（`faces`）的声明；两渠道共用的进程级 Ctrl+C（`signal`）。
//! 不管：业务单元、状态与不变式；两个渠道彼此不依赖，只共享 `faces` 这一份面袋与 `signal` 这一份控制台事件。
//! 联动：两者唯一共享的是各单元的 `api`；Web 资产与路由契约见 `docs/presentation/contracts.md`。
pub mod cli;
pub mod faces;
pub mod signal;
pub mod web;
