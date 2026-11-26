//! 目的：会话的落盘策略——哪种会话留档、哪些事件算定稿、系统会话的 sid 约定。
//! 管：`PersistPolicy` / `is_system_session` / `SYSTEM_SID_SUGGEST` / `persist_policy_for`。
//! 不管：流水怎么写到盘上（`HistoryStore` 端口与 `Persister`）、什么时候写（调用方）。
//! 联动：`session::service` 的 `persist_events` 按这里的策略过滤；`#` 开头的系统会话只推不留。

use crate::units::business::transcript::api::SessionEvent;

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
                // 原始 Notice 是**瞬时提示**（没有行 id，无法回档）：只推给在场的前端，
                // 不落盘。真正可回放的内容提示是 Transcript 里的 `LineView kind=notice`。
                | SessionEvent::Notice(_)
        )
    }
}

/// 目的：**系统会话的 sid 约定**：以 `#` 开头的是"没有地方落盘"的一次性动作（只推不留）。
///  见 docs/session/session-model.md 二（会话种类 × 推 / 落）。
pub fn is_system_session(sid: &str) -> bool {
    sid.starts_with('#')
}

/// 目的：核心推荐（suggest_models）的系统会话 sid：一次性的推荐动作**没有工作区**，
///  它的行只推给在场的前端看（推荐是弹窗里的一次问答），不落盘，也不进会话列表。
pub const SYSTEM_SID_SUGGEST: &str = "#suggest";

/// 目的：这个会话的落盘策略——判定只有这一处（会话种类 → 策略）。
pub fn persist_policy_for(sid: &str) -> PersistPolicy {
    if is_system_session(sid) {
        PersistPolicy::Drop
    } else {
        PersistPolicy::Keep
    }
}
