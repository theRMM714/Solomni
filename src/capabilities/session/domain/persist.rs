//! 目的：转录的留档策略——哪种会话留档、哪些事件算定稿。
//! 管：`PersistPolicy`（由会话种类定，不由调用点定）、`is_system_session`（`#` 前缀约定）与 `persist_policy_for`。
//! 不管：怎么写盘（`ports::HistoryStore` 端口）、什么时候写（调用方的用例）。
//! 联动：`session/service.rs` 的 `persist_events` / `record_events` / `Persister` 都经这里取策略，判定只有这一处。

use crate::capabilities::session::api::SessionEvent;

/// 目的：**这个会话要不要留档**：由**会话种类**定，不由调用点定。
///  - `Keep`：定稿事件落盘（单 agent 会话与协作会话都要回放——前者是模型上下文的一部分，
///    后者是派生状态的来源）；
///  - `Drop`：只推不留（**系统会话**：一次性动作，如"让核心推荐 agent"，重启后本来就该是空的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistPolicy {
    Keep,
    Drop,
}

impl PersistPolicy {
    /// 目的：一条事件该不该留：`Drop` 一条都不留；`Keep` 也不留**短暂事件**
    ///  （流式增量 / 运行态 / 实时工具卡——它们只给在场的前端看；裁决卡与回答按会话种类落盘）。
    /// 参数：`ev` 待判的事件。
    /// 返回：`true` = 落盘。
    pub fn keeps(&self, ev: &SessionEvent) -> bool {
        if *self == PersistPolicy::Drop {
            return false;
        }
        !matches!(
            ev,
            SessionEvent::Delta { .. }
                | SessionEvent::Working { .. }
                | SessionEvent::ToolCall(_)
                | SessionEvent::Rewound { .. }
                // 原始 Notice 是**瞬时提示**（没有行 id，无法回档）：只推给在场的前端，不落盘。
                | SessionEvent::Notice(_)
        )
    }
}

/// 目的：**系统会话的 sid 约定**：以 `#` 开头的是"没有地方落盘"的一次性动作（只推不留）。
///  见 docs/session/session-model.md 二（会话种类 × 推 / 落）。
/// 参数：`sid` 会话 id。
/// 返回：`true` = 系统会话（不进会话表、不落盘）。
pub fn is_system_session(sid: &str) -> bool {
    sid.starts_with('#')
}

/// 目的：这个会话的留档策略（**判定只有这一处**：会话种类 → 策略）。
/// 参数：`sid` 会话 id。
/// 返回：系统会话为 `Drop`，其余为 `Keep`。
pub fn persist_policy_for(sid: &str) -> PersistPolicy {
    if is_system_session(sid) {
        PersistPolicy::Drop
    } else {
        PersistPolicy::Keep
    }
}
