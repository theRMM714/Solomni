//! 目的：运行时的纯逻辑与值形态——会话对象与簿记、工具面的活对象、席位到工具面的装配、回档算术、消息回灌。
//! 管：`assemble`（角色 → 工具面）、`policy`（这一次调用准不准）、`assembly`（工具面与会话对象的整场装配）、`member_tools`（工具面活对象与核实环境）、`ports`（装配一席的端口参数对象）、`session`（会话状态、簿记、提示词）、`rewind`（回档算术）、`reply`（消息回灌）、`verify`（核心核实环境）。
//! 不管：IO 与端口实现（由装配方注入）；会话身份与声明（在 `session_model`）；裁决与流程（在会话单元）。
//! 联动：`engine` 只读写 `session::AgentSession` 的字段与方法；`api.rs` 对外重导出。

pub mod assemble;
pub mod assembly;
pub mod member_tools;
pub mod policy;
pub mod ports;
pub mod reply;
pub mod rewind;
pub mod session;
pub mod sources;
pub mod verify;
