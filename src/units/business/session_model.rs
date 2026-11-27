//! 目的：会话的身份与声明——"这条会话是什么、还该不该被驱动"的落盘事实与派生规则。
//! 管：单元的模块根（api 入站面 + domain 定义 + detail 装载）：会话元信息、会话类型与流程段声明、落盘策略。
//! 不管：会话对象与回合引擎（在会话单元）；落盘的 IO（在会话单元的 detail）；流程调度（在 collab / turn）；工具面合成与自洽断言（在会话单元的 assemble / loop_check）。
//! 联动：会话单元的装配与会话树、work 建工作、turn 的回合环境、runtime 的列表都经 `api.rs` 取事实；契约见 docs/session/session-model.md。

pub mod api;
pub mod detail;
pub mod domain;
