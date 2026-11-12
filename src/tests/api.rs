//! 目的：入站契约（conductor::api）的契约测试——命令/事件模型、能力分面、停止语义、动作目录、压缩与回档。
//! 管：核心手柄的命令面与动作面、事件事实与历史合并、停止/压缩/回档语义、动作目录与调用者校验。
//! 不管：HTTP 路由（本目录 routes）、协作回合内部（`src/tests/collab/`）、会话领域规则（`src/tests/session/`）。
//! 联动：`src/capabilities/conductor/api/` 与 `src/capabilities/conductor/service/`。

mod collab;
mod compaction;
mod core;
mod events;
mod guest;
mod session;
mod support;
mod tools;
