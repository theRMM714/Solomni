//! 目的：协作能力的用例与编排——讨论 / 执行 / 验收引擎与协作会话状态机。
//! 管：service 子模块的声明与归属（collab / discussion / driver / pump / round / slate / synthesis / tool_loop / turn_io）。
//! 不管：纯派生（domain/collab_state.rs）；对外导出（在 api.rs）；IO 机制本身（在对应能力的端口后）。
//! 联动：驱动一律经 session / llm / tools / workspace / registry / prompt 的声明面；对外只经 api.rs。

pub mod collab;
pub mod discussion;
pub mod driver;
pub mod pump;
pub mod round;
pub mod slate;
pub mod synthesis;
pub mod tool_loop;
pub mod turn_io;
