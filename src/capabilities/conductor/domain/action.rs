//! 目的：动作的纯逻辑——回包形状的拼装（不碰端口、不碰会话）。
//! 管：探测结论 → 响应 JSON（三种结论如实给出，不改写、不降级）。
//! 不管：怎么探测（在 registry / llm 的能力面）；谁在什么时机调（在 api 的分发里）。
//! 联动：由 `conductor/api/action.rs` 调用；结论类型归 `llm::api::ProbeOutcome`。

use crate::capabilities::llm::api::{ProbeOutcome, ToolMode};

/// 目的：探测结论 → 响应 JSON；`mode` 是探测后登记处里的**实际**形态（None = 取不到）。
/// 约束：结论只翻译、不解释——三种取值原样穿过，呈现层与前端不各自改写。
pub fn probe_view(outcome: &ProbeOutcome, mode: Option<ToolMode>) -> serde_json::Value {
    let (kind, detail) = match outcome {
        ProbeOutcome::Supported { detail } => ("supported", detail.clone()),
        ProbeOutcome::Unsupported { detail } => ("unsupported", detail.clone()),
        ProbeOutcome::Unknown { detail } => ("unknown", detail.clone()),
    };
    serde_json::json!({ "ok": true, "outcome": kind, "detail": detail, "mode": mode })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三种结论**原样**穿过（不改写、不降级）；`mode` 取登记处的实际值，不由结论反推。
    #[test]
    fn probe_view_reports_every_verdict_verbatim() {
        let cases = [
            (
                ProbeOutcome::Supported {
                    detail: "真的调了".to_string(),
                },
                "supported",
                "真的调了",
            ),
            (
                ProbeOutcome::Unsupported {
                    detail: "供应商说 tools 不认识".to_string(),
                },
                "unsupported",
                "供应商说 tools 不认识",
            ),
            (
                ProbeOutcome::Unknown {
                    detail: "没发起调用".to_string(),
                },
                "unknown",
                "没发起调用",
            ),
        ];
        for (outcome, want, detail) in cases {
            let v = probe_view(&outcome, Some(ToolMode::Envelope));
            assert_eq!(v["outcome"], want);
            assert_eq!(v["detail"], detail);
            assert_eq!(v["mode"], "envelope", "形态取登记处现有值: {}", v);
        }
        let no_mode = probe_view(
            &ProbeOutcome::Unknown {
                detail: "x".to_string(),
            },
            None,
        );
        assert!(no_mode["mode"].is_null());
    }
}
