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
    let ev = p.event("d3", "我建议先做 A", &[]).to_json();
    assert_eq!(ev, p.to_json("d3", "我建议先做 A", &[]), "推与快照同源");
    assert_eq!(
        ev.get("waiting")
            .and_then(|w| w.as_array())
            .map(|a| a.len()),
        Some(0),
        "队列里只有它一张：等待者清单为空"
    );
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

/// 目的：整队按转录重建：**没人回答也没作废**的卡按先来后到排着，答过的不重问。
#[test]
pub(crate) fn open_queue_rebuilds_every_unanswered_card_in_order() {
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
    let void = |cards: &[&str]| {
        serde_json::json!({
            "type": "decision_void",
            "cards": cards,
            "reason": "停止",
        })
    };
    let ids = |st: &collab_state::CollabState| -> Vec<String> {
        st.open_gates.iter().map(|(id, _, _)| id.clone()).collect()
    };
    // 三条卡，答掉第一条、作废第三条：队列里只剩 d2，且卡号计数不因重推而虚高。
    let events = vec![
        card("d1"),
        answer("d1", OPT_PLAN_SAY),
        card("d2"),
        card("d3"),
        void(&["d3"]),
    ];
    let st = collab_state::derive(&events, &[]);
    assert_eq!(st.cards, 3, "卡号计数器按已发出的卡数续号");
    assert_eq!(
        ids(&st),
        vec!["d2".to_string()],
        "答过的不重问、作废的不再挂"
    );
    // 队列形态变了会**重推队首那张**（同一卡号）：认出来就地更新，不重开一关、也不多算一个卡号。
    let mut again = events.clone();
    again.push(card("d2"));
    let st2 = collab_state::derive(&again, &[]);
    assert_eq!(st2.cards, 3, "同一张卡重推不算新卡：{:?}", ids(&st2));
    assert_eq!(ids(&st2), vec!["d2".to_string()]);
    // 两张都还没答：整队都在，顺序不乱（先来后到）。
    let st3 = collab_state::derive(&events[..4], &[]);
    assert_eq!(ids(&st3), vec!["d2".to_string(), "d3".to_string()]);
    // 队首那条事件**自带等待者的机制材料**：整队只凭转录就重建得回来（不是只剩队首一张），
    // 且重建出来的还是能问的那两关（不是认不出的空壳）。
    let head = serde_json::json!({
        "type": "decision_card",
        "id": "d1",
        "gate": "confirm_begin",
        "payload": { "advice": "先做 A" },
        "waiting": [{
            "id": "d2",
            "envelope": { "role": "core", "name": "核心" },
            "title": "现在开始讨论？",
            "gate": "confirm_begin",
            "payload": { "advice": "" },
        }],
    });
    let st4 = collab_state::derive(&[head], &[]);
    assert_eq!(
        ids(&st4),
        vec!["d1".to_string(), "d2".to_string()],
        "队首 + 后面还在等的：整队都在，顺序不乱"
    );
    assert_eq!(st4.cards, 2, "等待者也算发出去过的卡号（不复用）");
    let gates = crate::capabilities::collab::service::pump::derive_gates(&st4);
    assert_eq!(gates.len(), 2, "两关都要重建出来：{:?}", gates);
    assert_eq!(gates[1].0.as_deref(), Some("d2"), "第二关带自己的卡号");
    assert!(matches!(gates[1].1, Pending::ConfirmBegin));
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
        .open_queue(&sid)
        .expect("取卡")
        .expect("挂着一张卡")
        .card;
    // 旧卡的答案放行不了新请求：卡号必须是**当时那张**。
    let stale = core.collab_answer(&sid, "d404", OPT_BEGIN, "");
    assert!(stale.is_err(), "卡号对不上要拒：{:?}", stale);
    // 选项不在那张卡的选项集里：拒。
    let foreign = core.collab_answer(&sid, &card.id, OPT_PLAN_START, "");
    assert!(foreign.is_err(), "选项不在卡上要拒：{:?}", foreign);
    // 被拒的回答**不留痕**：这一关还挂着，卡号没变。
    let still = core.open_queue(&sid).unwrap().expect("还挂着").card;
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
    let first = core.open_queue(&sid).unwrap().expect("挂着一张卡").card;
    // 回档到需求行之后：会话按**转录**重建（重启同一条路）。
    core.rewind(
        &sid,
        crate::capabilities::conductor::api::RewindTarget::Delete(1),
    )
    .unwrap();
    let rebuilt = core.open_queue(&sid).unwrap().expect("重建后卡还在").card;
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
        .open_queue(&sid)
        .unwrap()
        .expect("整理完停在方案待审")
        .card;
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

/// 目的：队列（先来后到）：第二个发起方只能排在队首后面，一次只有队首可答、答了才轮到它。
#[test]
pub(crate) fn a_second_gate_queues_behind_the_first() {
    let mut core = core_with(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".to_string()]),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let first = core.open_queue(&sid).unwrap().expect("挂着一张卡");
    assert_eq!(first.card.id, "d1", "开场那一关先问");
    assert!(first.waiting.is_empty(), "队首后面还没人等");
    // 第二个发起方（这里用「再写一次本次需求」这条真实入口）只能排到它后面。
    let ev = core.collab_set_task(&sid, "第二个需求").unwrap();
    let queued = core.open_queue(&sid).unwrap().expect("还挂着");
    assert_eq!(queued.card.id, "d1", "队首不变：先来的先问");
    assert_eq!(queued.waiting.len(), 1, "后来的如实排队");
    assert!(
        queued.waiting[0].title.contains("开始讨论"),
        "等待者要说清在等什么：{:?}",
        queued.waiting[0].title
    );
    // 推的事件也带同一份队列形态（刷新页面前后看到的是同一件事）。
    let pushed: Vec<serde_json::Value> = ev.iter().map(|e| e.to_json()).collect();
    let shown: Vec<&serde_json::Value> = pushed
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("decision_card"))
        .collect();
    assert_eq!(shown.len(), 1, "队列形态变了要重推队首那条：{:?}", pushed);
    assert_eq!(
        shown[0]
            .get("waiting")
            .and_then(|w| w.as_array())
            .map(|a| a.len()),
        Some(1),
        "推的事件要带「前面还排着几条」：{:?}",
        shown[0]
    );
    // 等待者的**重建材料**随之落档（卡号 / 机制名 / 载荷）：重启后整队建得回来，不是只剩队首一张。
    let waiters = shown[0]
        .get("waiting")
        .and_then(|w| w.as_array())
        .expect("事件要带等待者");
    assert_eq!(
        waiters[0].get("id").and_then(|v| v.as_str()),
        Some("d2"),
        "等待者带自己的卡号：{:?}",
        waiters[0]
    );
    assert_eq!(
        waiters[0].get("gate").and_then(|v| v.as_str()),
        Some("confirm_begin"),
        "等待者带这一关的机制名：{:?}",
        waiters[0]
    );
    // **整队重建**（重启走这条路）：还没答之前重建，队首与后面还在等的那张都在、顺序不乱。
    let (meta, events) = core.history_open("w").unwrap();
    let rebuilt = core.rebuild_session(&meta, &events).expect("按转录重建");
    let crate::capabilities::conductor::service::Session::Collab(c) = rebuilt else {
        panic!("重建出来的还该是协作会话");
    };
    let q = c.door.queue().expect("重建后队首还在");
    assert_eq!(q.card.id, "d1", "队首还是先来的那张");
    assert_eq!(q.waiting.len(), 1, "后面还在等的那张也建回来了");
    assert_eq!(q.waiting[0].id, "d2", "等待者的卡号不乱");
    // 排在后面的那张还不能答（只有队首可见可答）。
    let early = core.collab_answer(&sid, "d2", OPT_BEGIN, "");
    assert!(early.is_err(), "排队里的卡还不能答：{:?}", early);
    // 答了队首：后面那张升为队首（卡号不变），并如实推出来。
    let ev = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    assert!(
        ev.iter()
            .any(|e| matches!(e, SessionEvent::DecisionCard { card, .. } if card.id == "d2")),
        "队首推进要如实推那张卡：{:?}",
        ev.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
    let next = core.open_queue(&sid).unwrap().expect("轮到它了");
    assert_eq!(next.card.id, "d2", "先来后到：队首依次往前");
    assert!(next.waiting.is_empty(), "队列里只剩它一张");
    // **重建整队**（重启 / 回档走同一条路）：按转录重建出来的是**整条队**，不只是最后一张，顺序不乱。
    let (meta, events) = core.history_open("w").unwrap();
    let rebuilt = core.rebuild_session(&meta, &events).expect("按转录重建");
    let crate::capabilities::conductor::service::Session::Collab(c) = rebuilt else {
        panic!("重建出来的还该是协作会话");
    };
    let q = c.door.queue().expect("重建后队首还在");
    assert_eq!(q.card.id, "d2", "重建后队首仍是没答的那张");
    assert!(q.waiting.is_empty(), "答过的 d1 不重问：{:?}", q.card.id);
}

/// 目的：停会话**整队作废**（停止 = 拒绝）：等待方解开、作废落盘、重建后不复活、继续时续新卡。
#[test]
pub(crate) fn stopping_voids_the_whole_queue_and_persists_it() {
    let (handle, ops) = ops_with(vec![module_of("a")], Vec::new());
    let spec = crate::capabilities::conductor::api::WorkSpec {
        name: "w-void".to_string(),
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
    ops.sessions
        .set_task(&sid, "第二个需求")
        .expect("再写一次需求（第二个发起方）");
    // 快照也带队列形态：刷新页面照样看得到「前面还排着几条」。
    let history = ops.history.list().expect("列历史");
    let view = ops
        .sessions
        .session_views(&history)
        .expect("会话视图")
        .into_iter()
        .find(|v| v.sid == sid)
        .expect("这条会话在表里");
    let snap = view.pending.expect("快照要带 pending");
    assert_eq!(
        snap.get("waiting")
            .and_then(|w| w.as_array())
            .map(|a| a.len()),
        Some(1),
        "快照的 pending 与推的事件同形：{:?}",
        snap
    );
    // 用户按停止：整队作废。
    ops.sessions.stop(&sid);
    assert!(
        ops.sessions.open_queue(&sid).expect("取卡").is_none(),
        "作废之后没有挂起的卡（等待方解开，不是永远挂着）"
    );
    let on_bus: Vec<serde_json::Value> = ops
        .events
        .snapshot(Some(&sid), 0)
        .0
        .into_iter()
        .flat_map(|l| l.events.into_iter().map(|e| e.to_json()))
        .collect();
    let voids: Vec<&serde_json::Value> = on_bus
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("decision_void"))
        .collect();
    assert_eq!(voids.len(), 1, "整队作废要如实外送一条：{:?}", voids);
    let cards: Vec<String> = voids[0]
        .get("cards")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        cards,
        vec!["d1".to_string(), "d2".to_string()],
        "没答的卡一律作废、按先来后到"
    );
    // 作废落盘之后：按转录重建（重启 / 回档走同一条路）**作废过的卡不复活**——
    // 那一关还挂在状态上，所以重建出来的是**续的新卡号**（d3），不是 d1 / d2。
    let (meta, events) = ops.history.open(&sid).expect("读转录");
    let q = handle
        .call({
            let meta = meta.clone();
            let events = events.clone();
            move |core| {
                let s = core.rebuild_session(&meta, &events)?;
                match s {
                    crate::capabilities::conductor::service::Session::Collab(c) => {
                        Ok(c.door.queue())
                    }
                    _ => Err("重建出来的还该是协作会话".to_string()),
                }
            }
        })
        .expect("命令通道")
        .expect("那一关还挂在状态上：续一张新卡");
    assert_ne!(q.card.id, "d1", "作废过的卡不复活");
    assert_ne!(q.card.id, "d2", "作废过的卡不复活");
    assert!(
        q.card.options.iter().any(|o| o.id == OPT_BEGIN),
        "续的还是那一关（开始讨论）：{:?}",
        q.card.options
    );
    // 「继续」：那一关还挂在状态上，续一张**新卡**接着问（停止 = 拒绝，不是放行）。
    ops.sessions
        .continue_flow(&sid, crate::capabilities::conductor::api::Output::Final)
        .expect("停止之后必须能继续");
    let again = ops
        .sessions
        .open_queue(&sid)
        .expect("取卡")
        .expect("续一张新卡接着问");
    assert_ne!(again.card.id, "d1", "续的是新卡号，不复活作废过的卡");
    assert!(
        again.card.options.iter().any(|o| o.id == OPT_BEGIN),
        "还是同一关（开始讨论）：{:?}",
        again.card.options
    );
}
