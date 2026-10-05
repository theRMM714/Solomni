//! 协作状态派生、回档与改需求、核心操作的外送
use super::super::builders::*;
use super::super::prelude::*;
#[test]
pub(crate) fn collab_state_derive_and_withdraw() {
    use crate::capabilities::collab::domain::collab_state::derive;
    // 行按**结构化字段**造（种类 / 说话人 / 动词 / 正文），与生产写的线格式同源。
    let ev = |id: u64, kind: &str, speaker: &str, verb: &str, line: &str| serde_json::json!({"type":"transcript","lines":[{"id":id,"kind":kind,"speaker":speaker,"verb":verb,"line":line}]});
    let events = vec![
        ev(0, "user", "用户", "需求", "做个东西"),
        ev(1, "user", "用户", "开始", "yes"),
        ev(2, "round", "轮次", "", "2"),
        ev(3, "msg", "a", "agree", "同意"),
    ];
    let st = derive(&events, &["a".to_string()]);
    assert_eq!(st.task.as_deref(), Some("做个东西"));
    assert!(st.begun && !st.allow);
    assert_eq!(st.round, 2);
    assert!(st.agreed["a"] && st.closed, "全员同意即收敛");
    // 撤回该同意后：不再算同意、讨论不再收敛
    let mut withdrawn = events.clone();
    withdrawn.push(ev(4, "user", "用户", "撤回", "a"));
    let st2 = derive(&withdrawn, &["a".to_string()]);
    assert!(!st2.agreed["a"] && !st2.closed);
    // 代拟行只给人看：名单不由它派生（权威来源是 meta.agents）。
    let with_slate = vec![
        ev(
            0,
            "system",
            "代拟",
            "",
            "甲〈a〉→ m（对口）；乙（复用；补位）",
        ),
        ev(1, "user", "用户", "名单", "确认"),
    ];
    let st3 = derive(&with_slate, &["a".to_string()]);
    assert_eq!(st3.picked, vec!["a".to_string()], "名单仍来自 meta.agents");
    assert!(
        st3.slate.is_some() && st3.slate_confirmed,
        "只保留原文供展示"
    );
}

#[test]
pub(crate) fn collab_rewind_rebuilds_from_transcript_and_resume_waits_at_gate() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(collab_work("c", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    // 回档到需求行之后（保留前 1 行 = 需求行）：协作走「按转录重建」
    let replayed = core
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(1),
        )
        .unwrap();
    assert_eq!(
        replay_lines(&replayed),
        vec!["[用户:需求] 做个东西".to_string()]
    );
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))),
        "重建后仍等确认开始"
    );
    // 继续：未开始不由继续代劳，只提醒
    let ev = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(matches!(&ev[0], SessionEvent::Notice(n) if n.contains("裁决门")));
}

#[test]
pub(crate) fn collab_update_task_rewinds_and_latest_wins() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(collab_work("c", &["a"], false, "旧需求"))
        .unwrap()
        .sid;
    let replayed = core.update_task(&sid, "新需求").unwrap();
    assert_eq!(
        replay_lines(&replayed),
        vec![
            "[用户:需求] 旧需求".to_string(),
            "[用户:需求] 新需求".to_string()
        ]
    );
    // 派生以最后一条需求为准
    let (_, events) = core.history_open("c").unwrap();
    let st = crate::capabilities::collab::domain::collab_state::derive(&events, &["a".to_string()]);
    assert_eq!(st.task.as_deref(), Some("新需求"));
}

#[test]
pub(crate) fn core_operation_streams_its_text_to_the_facts_outlet() {
    // 核心操作的正文也**逐片上屏**（与成员、单 agent 同一条规则，见 session::api::core_operation）：
    // 替身只发一片正文，出口必须收到 start + text 两条 Delta：核心的正文逐片上屏、不整块蹦出。
    // 同时钉住"信封不当正文流"：正文片以 { 开头，外送的那片必须是空的。
    let raw = "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}";
    let core = core_with_gateway(
        vec![module_of("a")],
        AbortGateway {
            raw: raw.to_string(),
        },
    );
    let (_picks, rows) = core
        .suggest_models("做个东西", WorkMode::Collab)
        .expect("核心推荐");
    let deltas: Vec<(String, String)> = rows
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Delta { kind, text, .. } => Some((kind.clone(), text.clone())),
            _ => None,
        })
        .collect();
    assert!(
        deltas.iter().any(|(k, _)| k == "start"),
        "核心要按轮起片：{:?}",
        deltas
    );
    assert!(
        deltas.iter().any(|(k, _)| k == "text"),
        "核心的正文要逐片上屏：{:?}",
        deltas
    );
    assert!(
        deltas.iter().all(|(k, t)| k != "text" || t.is_empty()),
        "工具信封绝不当正文流上屏：{:?}",
        deltas
    );
}

/// 讨论的调用参数**必须来自全局设置**（写死非流式会让协作卡住）。
#[test]
pub(crate) fn discussion_calls_carry_the_global_streaming_and_budget() {
    let llm = crate::capabilities::llm::api::LlmOpts {
        stream: true,
        timeout_secs: 123,
    };
    let (mut d, seen) = opts_discussion(vec![None, None, None], llm);
    assert!(
        d.open("任务", &mut |_, _| {}, &mut |_| {}).is_ok(),
        "开场正常"
    );
    let got = seen.lock().expect("锁").clone();
    assert_eq!(
        got[0],
        (true, 123),
        "开场调用要带上设置里的流式与预算：{:?}",
        got
    );
    let _ = d.step(&mut |_, _| {}, &mut |_| {});
    let got = seen.lock().expect("锁").clone();
    assert_eq!(got[1], (true, 123), "轮次调用同样：{:?}", got);
}

/// 调用失败：如实中断，**绝不把失败当发言吸收**（错误文本一旦进转录，"轮到谁"就歪了）。
#[test]
pub(crate) fn discussion_call_failure_interrupts_without_absorbing_a_line() {
    let (mut d, _seen) = opts_discussion(
        vec![None, Some("模型调用失败：超时".to_string()), None],
        Default::default(),
    );
    assert!(
        d.open("任务", &mut |_, _| {}, &mut |_| {}).is_ok(),
        "开场正常"
    );
    match d.step(&mut |_, _| {}, &mut |_| {}) {
        TurnOut::Interrupted(err) => assert!(err.contains("超时"), "原因要原样带回：{}", err),
        other => panic!(
            "失败必须中断，实际：{}",
            match other {
                TurnOut::Round => "Round",
                TurnOut::Done => "Done",
                TurnOut::AskUser { .. } => "AskUser",
                TurnOut::Interrupted(_) => "Interrupted",
                TurnOut::Stopped => "Stopped",
            }
        ),
    }
    // 开场那一次是正常的（留下一条发言）；失败那一次**不该**再添发言。
    let spoken = d.transcript.iter().filter(|l| l.speaker == "m0").count();
    assert_eq!(
        spoken,
        1,
        "失败不该被当成发言（只有开场那一条）：{:?}",
        d.transcript.iter().map(|l| l.render()).collect::<Vec<_>>()
    );
    assert!(
        d.transcript.iter().all(|l| !l.line.contains("超时")),
        "失败原因不该进转录"
    );
    assert!(!d.closed, "中断后讨论保持可继续（用户点「继续」重试）");
}

/// 链随 plan_review 事件派生：按转录重建时**不重新整理**（省一次模型调用）。
#[test]
pub(crate) fn collab_state_derives_the_task_chain_from_plan_review() {
    let events = vec![
        serde_json::json!({ "type": "plan", "text": "方案：A 做 X" }),
        serde_json::json!({
            "type": "plan_review",
            "plan": "方案：A 做 X",
            "chain": { "nodes": [ {
                "id": "n1", "title": "做 X", "objective": "把 X 做完",
                "assignee": "a", "deps": []
            } ] }
        }),
    ];
    let st = crate::capabilities::collab::domain::collab_state::derive(&events, &["a".to_string()]);
    assert_eq!(st.plan.as_deref(), Some("方案：A 做 X"));
    assert_eq!(st.chain.nodes.len(), 1, "链该从 plan_review 派生出来");
    assert_eq!(st.chain.nodes[0].assignee, "a");
    assert_eq!(st.chain.nodes[0].objective, "把 X 做完");
}

// ---------- 任务链（依赖图） ----------

/// 原生通道：供应商的结构化槽位 → 讨论动词；不认识的工具名 = 不认识（调用点据此**如实拒绝**）。
#[test]
pub(crate) fn native_tool_names_map_to_discussion_verbs() {
    use crate::capabilities::collab::service::round::{arg_text, verb_of};
    use crate::capabilities::llm::api::Verb;
    assert_eq!(verb_of("say"), Some(Verb::Say));
    assert_eq!(verb_of("agree"), Some(Verb::Agree));
    assert_eq!(verb_of("leave"), Some(Verb::Leave));
    assert_eq!(verb_of("ask"), Some(Verb::Ask));
    // 不是协作动词 = 不认识：讨论里出现这种调用就是越权，调用点会如实拒绝、不当表态吸收。
    assert_eq!(verb_of("read"), None);
    assert_eq!(verb_of("create_session"), None);
    // 参数里取正文；取不到就是空串（不猜）。
    assert_eq!(arg_text("{\"text\":\"同意\"}"), "同意");
    assert_eq!(arg_text("{}"), "");
    assert_eq!(arg_text("不是 JSON"), "");
}

/// 同意是**粘住**的：发过 agree 的人不再被追问（每轮重置等于每轮把所有人问一遍）。
#[test]
pub(crate) fn agreement_is_sticky_so_agreed_members_are_not_asked_again() {
    let mut d = scripted_discussion(
        vec![
            vec!["{\"type\":\"agree\",\"text\":\"同意\"}".into()],
            vec![
                "{\"type\":\"say\",\"text\":\"我补充\"}".into(),
                "{\"type\":\"say\",\"text\":\"再补充\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
            vec![
                "{\"type\":\"say\",\"text\":\"我也说\"}".into(),
                "{\"type\":\"say\",\"text\":\"还说\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
        ],
        false,
    );
    assert!(
        d.open("任务", &mut |_, _| {}, &mut |_| {}).is_ok(),
        "开场正常"
    );
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    // m0 在开场就同意了：之后每一轮都该跳过它（每轮重置同意会把它反复问一遍）。
    let asked_m0 = d.transcript.iter().filter(|l| l.speaker == "m0").count();
    assert_eq!(
        asked_m0,
        1,
        "发过 agree 的人不该被再问一次：{:?}",
        d.transcript.iter().map(|l| l.render()).collect::<Vec<_>>()
    );
}
