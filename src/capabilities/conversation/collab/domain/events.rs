//! 目的：协作自己的会话事件（**只属于协作**）：线上 kind/payload 与类型化视图的互转。
//! 管：`CollabEvent` 七种事件、`CheckView`、以及它们与 `SessionEvent::Push` 的双向映射。
//! 不管：事件的产生时机（在本能力的 `service/`）、会话通道本身（在会话能力）。
//! 联动：`service/` 造事件时用 `push`；呈现层与测试用 `of` 认回来（按 kind 解释，不认 Rust 枚举位置）。

use crate::capabilities::conversation::session::api::SessionEvent;
use crate::capabilities::taskchain::api::TaskChain;

/// 目的：验收条目的呈现视图。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckView {
    pub item: String,
    pub status: String,
    pub note: String,
}

/// 目的：协作的专有事件（会话通道只把它们当 kind + payload 转发）。
#[derive(Debug, Clone)]
pub enum CollabEvent {
    /// 讨论收敛（over_cap = 轮次超限，需用户裁决）。
    DiscussionDone { round: usize, over_cap: bool },
    /// 整理方案就绪。
    Plan(String),
    /// 方案待审（审查关卡）：把方案交给用户，等他点「同意」才开工。
    PlanReview { plan: String, chain: TaskChain },
    /// 任务链的一个节点开工：它跑在自己的**子会话**里（用户可进去干预）。
    NodeStarted {
        node: String,
        sid: String,
        assignee: String,
    },
    /// 成员执行回报（rework = 第几轮执行，0 为首轮）。
    Report {
        id: String,
        text: String,
        rework: usize,
    },
    /// 验收清单（raw = 解析失败时的原文）。
    Review { items: Vec<CheckView>, raw: String },
    /// 交付结论（over_rework = 返工超限交用户裁决）。
    Delivery { ok: bool, over_rework: bool },
}

impl CollabEvent {
    /// 目的：从线上 kind 与载荷认出这一条（渲染、测试与派生都走它，不猜字段顺序）。
    pub fn parse(kind: &str, payload: &serde_json::Value) -> Option<CollabEvent> {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let n = |k: &str| payload.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let b = |k: &str| payload.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        Some(match kind {
            "discussion_done" => CollabEvent::DiscussionDone {
                round: n("round"),
                over_cap: b("over_cap"),
            },
            "plan" => CollabEvent::Plan(s("text")),
            "plan_review" => CollabEvent::PlanReview {
                plan: s("plan"),
                chain: serde_json::from_value(payload.get("chain")?.clone()).ok()?,
            },
            "node_started" => CollabEvent::NodeStarted {
                node: s("node"),
                sid: s("sid"),
                assignee: s("assignee"),
            },
            "report" => CollabEvent::Report {
                id: s("id"),
                text: s("text"),
                rework: n("rework"),
            },
            "review" => CollabEvent::Review {
                items: serde_json::from_value(payload.get("items")?.clone()).ok()?,
                raw: s("raw"),
            },
            "delivery" => CollabEvent::Delivery {
                ok: b("ok"),
                over_rework: b("over_rework"),
            },
            _ => return None,
        })
    }

    /// 目的：从会话事件认回来：不是机制推送（或不是本能力的 kind）就是 None。
    pub fn of(event: &SessionEvent) -> Option<CollabEvent> {
        match event {
            SessionEvent::Push { kind, payload } => CollabEvent::parse(kind, payload),
            _ => None,
        }
    }

    /// 目的：装成会话事件（kind 即线上 type，载荷铺在顶层）。
    pub fn push(&self) -> SessionEvent {
        match self {
            CollabEvent::DiscussionDone { round, over_cap } => SessionEvent::push(
                "discussion_done",
                serde_json::json!({ "round": round, "over_cap": over_cap }),
            ),
            CollabEvent::Plan(text) => {
                SessionEvent::push("plan", serde_json::json!({ "text": text }))
            }
            CollabEvent::PlanReview { plan, chain } => SessionEvent::push(
                "plan_review",
                serde_json::json!({ "plan": plan, "chain": chain }),
            ),
            CollabEvent::NodeStarted {
                node,
                sid,
                assignee,
            } => SessionEvent::push(
                "node_started",
                serde_json::json!({ "node": node, "sid": sid, "assignee": assignee }),
            ),
            CollabEvent::Report { id, text, rework } => SessionEvent::push(
                "report",
                serde_json::json!({ "id": id, "text": text, "rework": rework }),
            ),
            CollabEvent::Review { items, raw } => {
                SessionEvent::push("review", serde_json::json!({ "items": items, "raw": raw }))
            }
            CollabEvent::Delivery { ok, over_rework } => SessionEvent::push(
                "delivery",
                serde_json::json!({ "ok": ok, "over_rework": over_rework }),
            ),
        }
    }
}
