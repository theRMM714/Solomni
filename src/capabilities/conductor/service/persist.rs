//! 目的：会话事件的落盘策略与增量落盘手柄——哪种会话留档、哪些事件算定稿、失败怎么报。
//! 管：`PersistPolicy` / `is_system_session` / `SYSTEM_SID_SUGGEST` / `persist_policy_for` / `persist_events` / `Persister`。
//! 不管：会话历史怎么存（session 的 HistoryStore 端口）、什么时候写（history.rs / 工作线程）。
//! 联动：history.rs 的 record_events 与工作线程的增量落盘都经这里；策略判定只有这一处。

use super::*;

/// 目的：**这个会话要不要留档**：由**会话种类**定，不由调用点定。
///  - `Keep`：定稿事件落盘（单 agent 会话与协作会话都要回放——前者是模型上下文的一部分，
///    后者是派生状态的来源）；
///  - `Drop`：只推不留（**系统会话**：一次性动作，如"让核心推荐 agent"，重启后本来就该是空的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistPolicy {
    Keep,
    Drop,
}

impl PersistPolicy {
    /// 目的：一条事件该不该留：`Drop` 一条都不留；`Keep` 也不留**短暂事件**
    ///  （流式增量 / 运行态 / 实时工具卡——它们只给在场的前端看；裁决卡与回答按会话种类落盘）。
    pub(crate) fn keeps(&self, ev: &SessionEvent) -> bool {
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
pub(crate) fn is_system_session(sid: &str) -> bool {
    sid.starts_with('#')
}

/// 目的：核心推荐（suggest_models）的系统会话 sid：一次性的推荐动作**没有工作区**，
///  它的行只推给在场的前端看（推荐是弹窗里的一次问答），不落盘，也不进会话列表。
pub(crate) const SYSTEM_SID_SUGGEST: &str = "#suggest";

/// 这个会话的落盘策略（**判定只有这一处**：会话种类 → 策略）。
fn persist_policy_for(sid: &str) -> PersistPolicy {
    if is_system_session(sid) {
        PersistPolicy::Drop
    } else {
        PersistPolicy::Keep
    }
}

/// 目的：事件落盘（按该会话的策略；短暂事件不落盘）：失败如实告知，返回要外送的警告（None = 一切正常）。
///  为什么抽成自由函数：工作线程按**一次模型调用**的粒度增量落盘，必须与核心走同一段逻辑——
///  两条路径各写一份的话，"什么算定稿、什么不落盘"迟早会不一致。
pub(crate) fn persist_events(
    history: &dyn History,
    log: &dyn Log,
    sid: &str,
    events: &[SessionEvent],
) -> Option<String> {
    // 留不留由**这个会话的策略**说了算（会话种类 → 策略，见 persist_policy_for）。
    let policy = persist_policy_for(sid);
    let jsons: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| policy.keeps(e))
        .map(|e| e.to_json())
        .collect();
    if jsons.is_empty() {
        return None;
    }
    match history.append(sid, &jsons) {
        Ok(()) => None,
        Err(e) => {
            log.error(
                "conductor::history_append",
                &format!("会话 {} 落盘失败：{}", sid, e),
            );
            Some(format!("[警告] 会话记录落盘失败：{}", e))
        }
    }
}

/// 目的：增量落盘手柄：工作线程按"一次模型调用"的粒度把定稿事件落盘。
///  为什么要它：整段生成跑完才落一次盘，会让中途刷新看不到已经产生的部分。
#[derive(Clone)]
pub(crate) struct Persister {
    pub(crate) history: Arc<dyn History + Send + Sync>,
    pub(crate) log: Arc<dyn Log + Send + Sync>,
    pub(crate) sid: String,
}

impl Persister {
    /// 目的：落盘这一批；返回要外送给用户的警告（None = 正常）。
    pub(crate) fn persist(&self, events: &[SessionEvent]) -> Option<String> {
        persist_events(self.history.as_ref(), self.log.as_ref(), &self.sid, events)
    }
}
