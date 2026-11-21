//! 目的：成员回合引擎——把一条 AgentSession 从"用户说了一句"驱动到一个完整回合（发言 / 派发 / 讨论发言 / 继续 / 压缩）。
//! 管：五个回合入口（`driver`）、轮循环与行构造（`round`）、工具循环与放行策略（`tools`）、回合端口与结论类型（`io`）、工具声明面的值类型（`decls`）。
//! 不管：会话状态与簿记（归 `domain`，引擎只读写它的字段与方法）；讨论状态机与协议反应（归协作能力）；模型与工具机制（在各自端口后）。
//! 联动：turn 能力的生成流程经 `AgentSession` 的方法驱动它；契约见 docs/session/session-model.md。

pub mod decls;
pub mod driver;
mod fence;
pub mod io;
pub mod round;
pub mod tools;
