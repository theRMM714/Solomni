//! 目的：钉 proxy 单元自己的纯判据——六工具入参的拒绝文案与默认值，全程不碰宿主、不碰会话。
//! 管：`observe_request` / `control_request` / `messages_request` 的必填、取值范围与默认值。
//! 不管：授权判据与幂等账（见 `tests/runtime/proxy.rs`）；宿主行为（`FakeProxyHost`）。
//! 联动：`proxy::api` 转出的纯函数，实现见 `src/capabilities/proxy/domain/proxy.rs`。

use crate::capabilities::proxy::api as d;

/// 目的：三个请求校验器在**任何宿主动作之前**就拒绝——文案逐字可断言，默认值与去空白规则固定。
#[test]
fn request_validators_reject_before_any_host_contact() {
    // observe：会话必填；view 只认四面；since 去空白后为空即视为"没有游标"。
    let empty = d::ObserveArgs {
        session_id: "  ".to_string(),
        view: "status".to_string(),
        since: None,
    };
    assert_eq!(
        d::observe_request(&empty).unwrap_err(),
        "session_id 不能为空"
    );

    let bad_view = d::ObserveArgs {
        session_id: " s1 ".to_string(),
        view: "messages".to_string(),
        since: None,
    };
    assert_eq!(
        d::observe_request(&bad_view).unwrap_err(),
        "view 只能是 status / pending / artifacts / full：messages"
    );

    let observed = d::ObserveArgs {
        session_id: " s1 ".to_string(),
        view: " artifacts ".to_string(),
        since: Some("  ".to_string()),
    };
    assert_eq!(
        d::observe_request(&observed).unwrap(),
        ("s1".to_string(), d::ObserveView::Artifacts, None)
    );

    // control：动作只认三个；reason 必填（它进可回放记录）；request_id 必填（幂等标识）。
    let base = |action: &str, reason: &str, request_id: &str| d::ControlArgs {
        session_id: "s1".to_string(),
        action: action.to_string(),
        reason: reason.to_string(),
        request_id: request_id.to_string(),
    };
    assert_eq!(
        d::control_request(&base("pause", "为什么", "req-1")).unwrap_err(),
        "action 只能是 stop / continue / close：pause"
    );
    assert_eq!(
        d::control_request(&base("close", "   ", "req-1")).unwrap_err(),
        "reason 不能为空（它会进可回放记录）"
    );
    assert_eq!(
        d::control_request(&base("close", "收工", "  ")).unwrap_err(),
        "request_id 不能为空（幂等标识）"
    );
    assert_eq!(
        d::control_request(&base(" close ", " 收工 ", " req-1 ")).unwrap(),
        (
            "s1".to_string(),
            d::ControlAction::Close,
            "收工".to_string(),
            "req-1".to_string()
        )
    );

    // messages：from 从 0 = 最新一条起倒查，不许为负；count 夹在 1..=50；两者都有默认值。
    let page = |from: Option<i64>, count: Option<i64>| d::MessagesArgs {
        session_id: "s1".to_string(),
        from,
        count,
    };
    assert_eq!(
        d::messages_request(&page(Some(-1), None)).unwrap_err(),
        "from 不能为负（0 = 最新一条）"
    );
    assert_eq!(
        d::messages_request(&page(None, Some(51))).unwrap_err(),
        "count 只能是 1 到 50"
    );
    assert_eq!(
        d::messages_request(&page(None, Some(0))).unwrap_err(),
        "count 只能是 1 到 50"
    );
    assert_eq!(
        d::messages_request(&page(None, None)).unwrap(),
        ("s1".to_string(), 0, 10)
    );
    assert_eq!(
        d::messages_request(&page(Some(3), Some(50))).unwrap(),
        ("s1".to_string(), 3, 50)
    );
}
