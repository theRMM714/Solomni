//! 跨业务共享的**事实类型**：只放没有领域逻辑的。
//! 见 docs/architecture/refactor-plan.md §1.3 R6：事实类型只属于 kernel，禁止各业务复制 DTO。

/// 前端唯一的会话标识 = 工作名（用户的命名，也是落盘目录名）。
pub type SessionId = String;
