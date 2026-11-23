//! 目的：钉住「驱动只认流程面」——一个与协作无关的替身流程跑在同一条驱动循环上。
//! 管：替身流程的装配、让出一步、吸收与收工、处置自己的关卡；成员回合仍由驱动按同一条轮循环跑。
//! 不管：真实协作流程（tests/collab）；会话表与落盘（turn / session 的用例）。
//! 联动：doubles/standin.rs 实现流程面；本文件不改 turn / work / runtime / lifecycle / presentation / session。

use super::prelude::*;
use crate::capabilities::conversation::collab::api::CollabSession;
use crate::capabilities::conversation::session::api::{
    DecisionCard, DecisionEnvelope, DecisionMessage, Gate, GateTicket, OptionView,
};
use crate::capabilities::turn::turn::api::Session;
use crate::tests::doubles::StandIn;

#[test]
pub(crate) fn a_standin_flow_runs_on_the_same_driver() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![r#"{"type":"say","text":"收到"}"#.to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec![]));
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    // 把在世会话换成替身流程：驱动除了流程面之外一个字段都不该看。
    let old = core.turn.take_flow_raw(&sid).expect("取在世会话");
    let systools = old
        .as_any()
        .downcast_ref::<CollabSession>()
        .expect("协作会话")
        .systools();
    core.turn.put(
        sid.to_string(),
        Session::Flow(Box::new(StandIn::new(systools))),
    );
    let events = core
        .turn
        .collab_advance(&sid, core.registry.as_ref(), core.probe.as_ref())
        .expect("推进替身流程");
    // ① 流程自己的事件照样从同一条通道出来（呈现侧按 Push 的 kind 派发）。
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Push { kind, .. } if kind == "standin")),
        "替身流程的事件没有出来：{:?}",
        events.len()
    );
    // ② 驱动替它把成员回合跑了：成员的话落在**它自己的会话**里。
    let child = format!("{}--a", sid);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(ls) if ls.iter().any(|l| l.line.contains("收到")))),
        "成员回合没有跑：{:?}",
        events.len()
    );
    let history = core.history_handle();
    let (_, lines) = history.load(&child).expect("成员会话落盘");
    assert!(
        lines
            .iter()
            .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("transcript")),
        "成员会话没有转录"
    );
    // ③ 流程吸收后自己宣布收工，驱动照它的判据停。
    assert!(
        events.iter().any(
            |e| matches!(e, SessionEvent::Push { kind, payload } if kind == "standin"
                && payload.get("step").and_then(|s| s.as_str()) == Some("fed"))
        ),
        "没有走到吸收那一步"
    );
    assert!(
        core.turn.session(&sid).is_some_and(|s| s.is_done()),
        "流程说了收工，会话却还没完成"
    );
}

#[test]
pub(crate) fn a_standin_flow_settles_its_own_gate() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![r#"{"type":"say","text":"收到"}"#.to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec![]));
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let old = core.turn.take_flow_raw(&sid).expect("取在世会话");
    let systools = old
        .as_any()
        .downcast_ref::<CollabSession>()
        .expect("协作会话")
        .systools();
    core.turn
        .put(sid.clone(), Session::Flow(Box::new(StandIn::new(systools))));
    let card = DecisionCard {
        id: "card-1".to_string(),
        envelope: DecisionEnvelope {
            role: "core".to_string(),
            name: "替身".to_string(),
        },
        message: DecisionMessage {
            title: "问一句".to_string(),
            body: "题面".to_string(),
            detail: String::new(),
        },
        options: vec![OptionView {
            id: "ok".to_string(),
            label: "好".to_string(),
        }],
    };
    let gate = Gate {
        kind: "standin".to_string(),
        payload: serde_json::json!({}),
        card,
        notes: Vec::new(),
    };
    // ① 就地收尾与不是这一关的判据都归流程：驱动只问、不认种类。
    assert!(
        core.turn.settles_inline(&sid, &gate),
        "流程说了这一关就地收尾"
    );
    let elsewhere = Gate {
        kind: "elsewhere".to_string(),
        ..gate.clone()
    };
    assert!(
        !core.turn.settles_inline(&sid, &elsewhere),
        "不是这一流程发的关就别插嘴"
    );
    // ② 把票交回流程面：处置由替身自己做，驱动接着推进同一条循环。
    let ticket = GateTicket {
        option: "ok".to_string(),
        note: String::new(),
        gate: Box::new(gate),
    };
    let events = core
        .turn
        .dispose_and_advance(&sid, &ticket, core.registry.as_ref(), core.probe.as_ref())
        .expect("处置替身的关卡");
    assert!(
        events.iter().any(
            |e| matches!(e, SessionEvent::Push { kind, payload } if kind == "standin"
                && payload.get("step").and_then(|s| s.as_str()) == Some("settled"))
        ),
        "关卡没有回到流程手里"
    );
    // ③ 流程收工后会话照常被摘掉。
    assert!(core.turn.session(&sid).is_none(), "流程收工后会话该被摘掉");
}
