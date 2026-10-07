//! 目的：裁决通道（卡片 + 选项）的契约测试：卡的派生、回答校验、落盘与重建、快照同形。
//! 管：` Pending ` 派生的卡片形状、选项 id 的稳定性、回答的唯一入口与校验、转录重建。
//! 不管：工具级确认（那一张卡本片仍走它自己的路）；真实 Web 渲染由 L4 端到端覆盖。
//! 联动：契约见 docs/session/session-model.md 的「请用户裁决：一条通道，消息 + 选项」。

use super::super::builders::answer_card;
use super::super::ops_with;
use super::super::prelude::*;

/// 一张卡上的选项 id（顺序即呈现顺序）。
fn option_ids(card: &DecisionCard) -> Vec<String> {
    card.options.iter().map(|o| o.id.clone()).collect()
}

/// 目的：五个门各自的卡片：id / 信封 / 消息 / 选项都在，且选项 id 是契约（改文案不改行为）。
#[test]
pub(crate) fn every_gate_derives_a_card_with_stable_option_ids() {
    let cases: Vec<(Pending, &str, Vec<String>)> = vec![
        (
            Pending::Ask {
                member: "甲".to_string(),
                question: "选哪个？".to_string(),
            },
            "member",
            vec![OPT_ASK_REPLY.to_string()],
        ),
        (
            Pending::ConfirmSlate,
            "core",
            vec![OPT_SLATE_CONFIRM.to_string(), OPT_SLATE_CANCEL.to_string()],
        ),
        (
            Pending::ConfirmBegin,
            "core",
            vec![OPT_BEGIN.to_string(), OPT_BEGIN_ALLOW.to_string()],
        ),
        (
            Pending::PlanReview,
            "core",
            vec![OPT_PLAN_START.to_string(), OPT_PLAN_SAY.to_string()],
        ),
        (
            Pending::NodeBlocked {
                nodes: vec!["n1-1".to_string()],
            },
            "core",
            vec![OPT_NODE_REWORK.to_string(), OPT_NODE_SAY.to_string()],
        ),
    ];
    for (p, role, ids) in cases {
        let card = p.card("d7", "建议：先做 A");
        assert_eq!(card.id, "d7", "卡号由发起方给：{:?}", p);
        assert_eq!(card.envelope.role, role);
        if role == "core" {
            assert_eq!(card.envelope.name, "核心");
        }
        assert!(!card.message.title.is_empty(), "标题不能空：{:?}", p);
        assert!(!card.message.body.is_empty(), "正文不能空：{:?}", p);
        let got = option_ids(&card);
        assert_eq!(got, ids, "{:?} 的选项集变了（选项 id 是契约）", p);
        assert!(
            card.options.iter().all(|o| !o.label.is_empty()),
            "每个选项都要有文案：{:?}",
            p
        );
    }
}

/// 目的：唯一派生——推的事件与快照是同一份 JSON；机制名 + 载荷能把这一关重建回来。
#[test]
pub(crate) fn event_and_snapshot_are_one_derivation_and_can_be_rebuilt() {
    let p = Pending::Ask {
        member: "甲".to_string(),
        question: "选哪个？".to_string(),
    };
    let ev = p.event("d3", "我建议先做 A").to_json();
    assert_eq!(ev, p.to_json("d3", "我建议先做 A"), "推与快照同源");
    assert_eq!(
        ev.get("type").and_then(|t| t.as_str()),
        Some("decision_card")
    );
    assert_eq!(ev.get("gate").and_then(|t| t.as_str()), Some(p.kind()));
    assert_eq!(ev.get("id").and_then(|t| t.as_str()), Some("d3"));
    // 建议随卡一起落档（重启重建要它，不能只活在内存里）。
    let payload = ev.get("payload").expect("载荷");
    assert_eq!(
        payload.get("advice").and_then(|a| a.as_str()),
        Some("我建议先做 A")
    );
    let again = Pending::from_payload(p.kind(), payload).expect("按机制名重建这一关");
    assert_eq!(again.card("d3", "").options.len(), 1);
    // 认不出的门如实给 None——不猜、不硬凑一张卡。
    assert!(Pending::from_payload("鬼门", payload).is_none());
}

/// 目的：挂起只看**最后一张没人回答的卡**：答过的不得重问。
#[test]
pub(crate) fn open_gate_follows_the_last_unanswered_card() {
    use crate::capabilities::collab::domain::collab_state;
    let card = |id: &str| {
        serde_json::json!({
            "type": "decision_card",
            "id": id,
            "gate": "plan_review",
            "payload": { "advice": "" },
        })
    };
    let answer = |id: &str, option: &str| {
        serde_json::json!({
            "type": "decision_answer",
            "card": id,
            "by": "用户",
            "option": option,
            "note": "",
        })
    };
    let events = vec![card("d1"), answer("d1", OPT_PLAN_SAY), card("d2")];
    let st = collab_state::derive(&events, &[]);
    assert_eq!(st.cards, 2, "卡号计数器按已发出的卡数续号");
    assert_eq!(
        st.open_gate.as_ref().map(|(id, _, _)| id.as_str()),
        Some("d2"),
        "挂的是最后一张没人回答的卡"
    );
    let mut answered = events.clone();
    answered.push(answer("d2", OPT_PLAN_START));
    let st2 = collab_state::derive(&answered, &[]);
    assert!(st2.open_gate.is_none(), "答过的不得重问");
}

/// 目的：回答只有一个入口：卡号对不上、选项不在那张卡上，都如实拒绝、不留痕。
#[test]
pub(crate) fn answers_are_validated_against_the_card_they_answered() {
    let mut core = core_with(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".to_string()]),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let card = core
        .collab_open_card(&sid)
        .expect("取卡")
        .expect("挂着一张卡");
    // 旧卡的答案放行不了新请求：卡号必须是**当时那张**。
    let stale = core.collab_answer(&sid, "d404", OPT_BEGIN, "");
    assert!(stale.is_err(), "卡号对不上要拒：{:?}", stale);
    // 选项不在那张卡的选项集里：拒。
    let foreign = core.collab_answer(&sid, &card.id, OPT_PLAN_START, "");
    assert!(foreign.is_err(), "选项不在卡上要拒：{:?}", foreign);
    // 被拒的回答**不留痕**：这一关还挂着，卡号没变。
    let still = core.collab_open_card(&sid).unwrap().expect("还挂着");
    assert_eq!(still.id, card.id);
    // 合法的回答：落一条"谁答的、选了哪个 id"的记录。
    let ev = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    assert!(
        ev.iter().any(|e| matches!(
            e,
            SessionEvent::DecisionAnswer(a) if a.card == card.id && a.option == OPT_BEGIN
        )),
        "回答要如实外送：{:?}",
        ev
    );
}

/// 目的：落盘与重建：卡与回答都进转录，重启（按转录重建）后卡还在。
#[test]
pub(crate) fn cards_and_answers_survive_a_rebuild() {
    let mut core = core_with(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".to_string()]),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let first = core.collab_open_card(&sid).unwrap().expect("挂着一张卡");
    // 回档到需求行之后：会话按**转录**重建（重启同一条路）。
    core.rewind(
        &sid,
        crate::capabilities::conductor::api::RewindTarget::Delete(1),
    )
    .unwrap();
    let rebuilt = core.collab_open_card(&sid).unwrap().expect("重建后卡还在");
    assert_eq!(rebuilt.id, first.id, "卡号跨重建稳定");
    let ids = option_ids(&rebuilt);
    let was = option_ids(&first);
    assert_eq!(ids, was, "重建出来的选项集与原卡一致");
    // 答过它：转录里既有卡也有回答（重启后仍能重建"答过什么"）。
    answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    let (_, events) = core.history_open("w").unwrap();
    let kinds: Vec<String> = events
        .iter()
        .filter_map(|e| e.get("type").and_then(|t| t.as_str()).map(String::from))
        .collect();
    assert!(
        kinds.iter().any(|k| k == "decision_card"),
        "卡要落档：{:?}",
        kinds
    );
    assert!(
        kinds.iter().any(|k| k == "decision_answer"),
        "回答要落档：{:?}",
        kinds
    );
}

/// 目的：快照里的 pending 与推来的那张卡是**同一个事实**（刷新页面照样画得出那张卡）。
#[test]
pub(crate) fn snapshot_pending_is_the_same_fact_as_the_pushed_card() {
    let (handle, ops) = ops_with(vec![module_of("a")], Vec::new());
    let spec = crate::capabilities::conductor::api::WorkSpec {
        name: "w-card".to_string(),
        mode: WorkMode::Collab,
        agents: vec![AgentInstance {
            name: "a".to_string(),
            transient: true,
            modules: vec!["a".to_string()],
            model: None,
        }],
        task: Some("做个东西".to_string()),
        delegate: false,
        tier: Tier::Host,
    };
    let opened = ops.sessions.create_work(spec).expect("建协作工作");
    let sid = opened.0.sid.clone();
    let pushed: Vec<serde_json::Value> = ops
        .events
        .snapshot(Some(&sid), 0)
        .0
        .into_iter()
        .flat_map(|l| l.events.into_iter().map(|e| e.to_json()))
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("decision_card"))
        .collect();
    assert_eq!(pushed.len(), 1, "开场该推一张卡：{:?}", pushed);
    let history = ops.history.list().expect("列历史");
    let view = ops
        .sessions
        .session_views(&history)
        .expect("会话视图")
        .into_iter()
        .find(|v| v.sid == sid)
        .expect("这条会话在表里");
    let snap = view.pending.expect("快照要带 pending");
    assert_eq!(snap, pushed[0], "推的卡与快照的 pending 必须一模一样");
    assert_eq!(
        snap.get("id").and_then(|i| i.as_str()),
        Some("d1"),
        "卡号会话内唯一、从 d1 起"
    );
    let _ = handle;
}

/// 目的："先说一句"这一条：他的话进主会话当反馈，由核心 AI 判是否明确——明确才开工。
#[test]
pub(crate) fn the_say_option_lets_the_core_judge_the_users_words() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let plan = "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string();
    let verdict = "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string();
    let node = "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string();
    let check = "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string();
    let mut core = core_with(
        vec![module_of("a")],
        gw(member, vec![plan, verdict, node, check]),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    let card = core
        .collab_open_card(&sid)
        .unwrap()
        .expect("整理完停在方案待审");
    // 附言为空 = 没有可判的话：如实拒绝，不拿空话当放行。
    assert!(
        core.collab_answer(&sid, &card.id, OPT_PLAN_SAY, "  ")
            .is_err(),
        "先说一句必须有话说"
    );
    let ev = answer_card(&mut core, &sid, OPT_PLAN_SAY, "照方案开工吧").unwrap();
    assert!(
        ev.iter()
            .any(|e| matches!(e, SessionEvent::DecisionAnswer(a) if a.option == OPT_PLAN_SAY)),
        "回答要落档：{:?}",
        ev
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("照你说的开工"))),
        "明确才开工：{:?}",
        ev
    );
}

/// 目的："开工"这一条：不花判定那一次调用，直接放行（脚本里没有 verdict 那一条）。
#[test]
pub(crate) fn the_start_option_releases_the_gate_without_a_judgement_call() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let plan = "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string();
    let node = "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string();
    let check = "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string();
    let mut core = core_with(vec![module_of("a")], gw(member, vec![plan, node, check]));
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    let events = answer_card(&mut core, &sid, OPT_PLAN_START, "按方案推进").unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "点开工就该一路跑到交付：{:?}",
        events.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
}
