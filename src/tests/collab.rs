//! 协作能力测试：讨论泵、任务链派发、审查关卡、节点验收、代拟。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

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
    let replayed = core.rewind(&sid, 1).unwrap();
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
    // 核心操作的正文也**逐片上屏**（与成员、单 agent 同一条规则，见 engine::core_operation）：
    // 替身只发一片正文，出口必须收到 start + text 两条 Delta——从前核心是一次性蹦出来的（真机反馈）。
    // 同时钉住"信封不当正文流"：正文片以 { 开头，外送的那片必须是空的。
    let raw = "{\"type\":\"tool\",\"name\":\"suggest\",\"args\":{\"agents\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}";
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

/// 讨论的调用参数**必须来自全局设置**（以前这里写死非流式，正是协作卡住的成因之一）。
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
    use crate::capabilities::collab::domain::engine::{arg_text, verb_of};
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

/// 同意是**粘住**的：发过 agree 的人不再被追问（以前每轮重置，等于每轮把所有人问一遍）。
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
    // m0 在开场就同意了：之后每一轮都该跳过它（以前每轮重置同意，会把它反复问一遍）。
    let asked_m0 = d.transcript.iter().filter(|l| l.speaker == "m0").count();
    assert_eq!(
        asked_m0,
        1,
        "发过 agree 的人不该被再问一次：{:?}",
        d.transcript.iter().map(|l| l.render()).collect::<Vec<_>>()
    );
}

#[test]
pub(crate) fn discussion_full_agreement() {
    let mut d = scripted_discussion(
        vec![
            vec![
                "{\"type\":\"say\",\"text\":\"好\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
            vec![
                "{\"type\":\"say\",\"text\":\"行\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
        ],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    assert!(d
        .transcript
        .iter()
        .any(|l| l.speaker == "m0" && l.verb == "agree"));
}

#[test]
pub(crate) fn discussion_ask_pauses() {
    let mut d = scripted_discussion(
        vec![vec!["{\"type\":\"ask\",\"text\":\"需要参数?\"}".into()]],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    match d.step(&mut |_, _| {}, &mut |_| {}) {
        TurnOut::AskUser { member, question } => {
            assert_eq!(member, "m0");
            assert_eq!(question, "需要参数?");
        }
        _ => panic!("应暂停请教"),
    }
}

#[test]
pub(crate) fn discussion_leave_shrinks() {
    let mut d = scripted_discussion(
        vec![vec!["{\"type\":\"leave\",\"text\":\"撤了\"}".into()]],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    let _ = d.step(&mut |_, _| {}, &mut |_| {});
    assert!(d.members.iter().all(|m| !m.present));
}

#[test]
pub(crate) fn discussion_autonomy_archives_ask() {
    let mut d = scripted_discussion(
        vec![vec![
            "{\"type\":\"say\",\"text\":\"开场\"}".into(),
            "{\"type\":\"ask\",\"text\":\"细节?\"}".into(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
        ]],
        true,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("自裁模式不该暂停"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.line.contains("自裁")));
}

#[test]
pub(crate) fn discussion_round_cap_enforced() {
    let mut d = scripted_discussion(
        vec![vec!["{\"type\":\"say\",\"text\":\"继续\"}".into()]; 2],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    assert!(d.round > MAX_ROUNDS);
}

/// 讨论回合能**先核实再发言**：只读工具真跑（核实行带工具视图），随后照常表态。
#[test]
pub(crate) fn discussion_member_can_inspect_before_speaking() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            // 先核实一次：路径不合法 → 工具**如实失败**，但它确实跑了、留了核实行。
            "{\"type\":\"tool\",\"name\":\"list\",\"args\":{\"path\":\"x\"}}".to_string(),
            "{\"type\":\"say\",\"text\":\"查过了\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let inspected = events.iter().any(|e| {
        matches!(e, SessionEvent::Transcript(ls)
            if ls.iter().any(|l| l.tool.as_ref().map(|t| t.name.as_str() == "list").unwrap_or(false)))
    });
    assert!(
        inspected,
        "讨论里的核实要留下**带工具视图**的行：{:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(ls)
            if ls.iter().any(|l| l.line.contains("查过了")))),
        "核实之后要能正常发言：{:?}",
        events
    );
}

/// 讨论回合**按角色表校验**：面里没有的内置工具（write）被如实拒绝，不当表态吸收。
#[test]
pub(crate) fn discussion_member_cannot_call_a_builtin_outside_its_role_face() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"x\",\"content\":\"y\"}}"
                .to_string(),
            "{\"type\":\"say\",\"text\":\"我不该写文件\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Notice(n)
            if n.contains("[越权]") && n.contains("write"))),
        "角色表没发给讨论席的工具该被如实拒绝：{:?}",
        events
    );
}

/// 讨论回合**拿不到干活的手段**：模块工具被如实拒绝，且不当表态吸收。
#[test]
pub(crate) fn discussion_member_cannot_use_module_tools() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"tool\",\"name\":\"harvest.scan\",\"args\":{}}".to_string(),
            "{\"type\":\"say\",\"text\":\"我不该干活\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Notice(n)
            if n.contains("[越权]") && n.contains("harvest.scan"))),
        "模块工具在讨论回合该被如实拒绝：{:?}",
        events
    );
}

/// 讨论席的一回合：**逐轮定稿**（核实行与发言行各是各的一条落档事件，不是攒到回合末一条），
/// 并且**实时落的行与回档重建出的对话同口径**——重启后子会话的上下文不歪
/// （回档按行截断历史，靠的就是这两边一致；见 docs/architecture/session-model.md 二之二、五）。
#[test]
pub(crate) fn discussion_member_turn_finalizes_by_round_and_rebuilds_the_same_dialogue() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            // 先核实一次（读数），再表态（收尾）。
            "{\"type\":\"tool\",\"name\":\"list\",\"args\":{\"path\":\".\"}}".to_string(),
            "{\"type\":\"say\",\"text\":\"看过了\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let child = format!("{}--a", sid);
    // ① 逐轮定稿：核实行与发言行是两条 Transcript，且同属一个回合号。
    let (_, rows) = core.history_open(&child).unwrap();
    let batches: Vec<Vec<serde_json::Value>> = rows
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .map(|ls| ls.to_vec())
        .filter(|b: &Vec<serde_json::Value>| !b.is_empty())
        .collect();
    let tool_at = batches
        .iter()
        .position(|b| b.iter().any(|l| l.get("tool").is_some()))
        .expect("核实行该落档");
    let said_at = batches
        .iter()
        .position(|b| {
            b.iter()
                .any(|l| l.get("verb").and_then(|v| v.as_str()) == Some("say"))
        })
        .expect("发言行该落档");
    let turn_of = |b: &Vec<serde_json::Value>, pick: &dyn Fn(&serde_json::Value) -> bool| {
        b.iter()
            .find(|l| pick(l))
            .and_then(|l| l.get("turn").and_then(|t| t.as_u64()))
    };
    let tool_turn = turn_of(&batches[tool_at], &|l| l.get("tool").is_some());
    let said_turn = turn_of(&batches[said_at], &|l| {
        l.get("verb").and_then(|v| v.as_str()) == Some("say")
    });
    assert!(
        tool_at < said_at,
        "一轮一条：核实行先出、发言行后出（不是回合末一次性一批）：{batches:?}"
    );
    assert_eq!(tool_turn, said_turn, "同一个回合的行带同一个回合号");
    // ② 实时与重建同口径：把会话从表里丢掉，再取一次 = 按落盘转录重建。
    let shown = |s: &crate::capabilities::session::api::AgentSession| {
        s.dialogue()
            .iter()
            .map(|m| format!("{}:{}", m.role, m.content))
            .collect::<Vec<_>>()
    };
    let live = core.take_single(&child).expect("会话在表里");
    let live_msgs = shown(&live);
    // 丢掉会话本体 + 撤下"生成中"：这一步之后的取用只能**按落盘转录重建**（崩溃/重启同一条路）。
    drop(live);
    core.abort_running(&child);
    core.prepare_single(&child, None, false)
        .expect("按落盘转录重建（ensure_session 这条路）");
    let rebuilt = core.take_single(&child).expect("取回重建出来的会话");
    assert_eq!(
        shown(&rebuilt),
        live_msgs,
        "实时与重建必须逐条一致（否则重启后子会话的上下文就歪了）"
    );
}

/// **没写信封的原文不算表态**：不投影主会话；提醒到顶才留一行**系统消息**"未回应"。
/// 判据是结构化的（system / degraded 字段），呈现层不靠匹配文案。
#[test]
pub(crate) fn prose_without_an_envelope_is_not_a_statement() {
    let members = vec![Member::new(
        "m0",
        crate::tests::doubles::test_params("m0"),
        crate::capabilities::llm::api::ToolMode::Envelope,
        scripted(vec!["我觉得可以".into()]),
    )];
    let mut disc = Discussion::new(
        members,
        true,
        std::sync::Arc::new(test_prompts()),
        test_tools_svc(),
        Default::default(),
        Default::default(),
        String::new(),
    );
    let _ = disc.open("任务", &mut |_, _| {}, &mut |_| {});
    assert!(
        !disc
            .transcript
            .iter()
            .any(|l| l.speaker == "m0" && l.verb == "say"),
        "散文不是表态，不该投影主会话：{:?}",
        disc.transcript
            .iter()
            .map(|l| l.render())
            .collect::<Vec<_>>()
    );
    let note = disc
        .transcript
        .iter()
        .find(|l| l.line.contains("未回应"))
        .expect("提醒到顶该记一行未回应");
    assert!(
        note.system,
        "未回应是**系统消息**：结构化标记，呈现层不靠匹配文案"
    );

    // 线格式：只在为真时写出这两个字段
    let yes = SessionEvent::Transcript(vec![crate::capabilities::session::api::LineView {
        id: 0,
        line: "x".into(),
        system: true,
        degraded: true,
        ..Default::default()
    }])
    .to_json();
    assert_eq!(yes["lines"][0]["system"], serde_json::Value::Bool(true));
    assert_eq!(yes["lines"][0]["degraded"], serde_json::Value::Bool(true));
    let no = SessionEvent::Transcript(vec![crate::capabilities::session::api::LineView {
        id: 0,
        line: "x".into(),
        ..Default::default()
    }])
    .to_json();
    assert!(
        no["lines"][0].get("system").is_none() && no["lines"][0].get("degraded").is_none(),
        "非系统/非降级行不写这些字段"
    );
}

// ---------- 执行/验收 ----------

#[test]
pub(crate) fn execution_review_pass_and_fail_paths() {
    let prompts = test_prompts();
    let mut members = vec![Member::new(
        "m0",
        crate::tests::doubles::test_params("m0"),
        crate::capabilities::llm::api::ToolMode::Envelope,
        scripted(vec!["{\"type\":\"say\",\"text\":\"汇报内容\"}".into()]),
    )];
    let ran = run_execution(members.as_mut_slice(), "任务A", &prompts);
    assert_eq!(ran.reports.get("m0").map(|s| s.as_str()), Some("汇报内容"));

    // 验收：核心对照方案逐项核对（总验收用的就是这条）。
    let mut exec = crate::capabilities::collab::domain::engine::Execution::new();
    exec.reports = ran.reports.clone();
    let mut core_chat = scripted(vec![
        "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"A\",\"status\":\"fail\",\"reason\":\"没做完\"}]}}".into(),
    ]);
    exec.review(
        core_chat.as_mut(),
        "方案",
        "",
        None,
        &prompts,
        &*test_tools_svc(),
        Default::default(),
        Default::default(),
        None,
        &mut |_e: crate::capabilities::session::api::SessionEvent| {},
    );
    assert!(!exec.all_pass(), "有 fail 项就不通过");

    // 再验一次：这次全 pass。
    let mut exec2 = crate::capabilities::collab::domain::engine::Execution::new();
    exec2.reports = ran.reports.clone();
    let mut core_chat2 = scripted(vec![
        "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"A\",\"status\":\"pass\"}]}}"
            .into(),
    ]);
    exec2.review(
        core_chat2.as_mut(),
        "方案",
        "",
        None,
        &prompts,
        &*test_tools_svc(),
        Default::default(),
        Default::default(),
        None,
        &mut |_e: crate::capabilities::session::api::SessionEvent| {},
    );
    assert!(exec2.all_pass());
}

#[test]
pub(crate) fn review_parse_failure_is_conservative_fail() {
    let prompts = test_prompts();
    let mut members = vec![Member::new(
        "m0",
        crate::tests::doubles::test_params("m0"),
        crate::capabilities::llm::api::ToolMode::Envelope,
        scripted(vec!["{\"type\":\"say\",\"text\":\"x\"}".into()]),
    )];
    let mut exec = crate::capabilities::collab::domain::engine::Execution::new();
    exec.reports = run_execution(members.as_mut_slice(), "任务", &prompts).reports;
    let mut core_chat = scripted(vec!["完全不是清单".to_string()]);
    exec.review(
        core_chat.as_mut(),
        "方案",
        "",
        None,
        &prompts,
        &*test_tools_svc(),
        Default::default(),
        Default::default(),
        None,
        &mut |_e: crate::capabilities::session::api::SessionEvent| {},
    );
    assert!(exec.items.is_empty());
    assert!(!exec.all_pass(), "解析失败必须保守判否");
}

// ---------- Core 门面：会话中心（内存组合根） ----------

#[test]
pub(crate) fn mode_vocabulary_is_single_or_collab_only() {
    // web 与 core 的形态词汇只有 single / collab；旧的 direct / compose 一概不认（GREEN FIELD，无兼容）。
    use crate::web::parse_mode;
    assert!(matches!(parse_mode("single"), Ok(WorkMode::Single)));
    assert!(matches!(parse_mode("collab"), Ok(WorkMode::Collab)));
    for bad in ["direct", "compose", "omni", ""] {
        let e = parse_mode(bad).unwrap_err();
        assert!(e.contains("未知模式"), "web 必须 400 并说明：{}", e);
    }
    // 落盘 meta 里的旧形态不做兼容：重建会话时明确报错（不静默当单 agent）。
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    hist.create(&SessionMeta {
        name: "旧会话".to_string(),
        mode: "direct".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 1,
        agents: vec![AgentMeta {
            name: "a".to_string(),
            transient: true,
            modules: vec!["a".to_string()],
            model: None,
        }],
        exec: ExecSpec::default(),
        parent: None,
        node: None,
    })
    .unwrap();
    // 内存里没有这个会话 → 走 rebuild_session，对未知形态如实报错。
    let err = core.rewind("旧会话", 0).unwrap_err();
    assert!(err.contains("未知会话形态"), "{}", err);
}

/// 方案过审后：就绪节点各建一个**子会话**（普通单 agent 会话，沙箱锚在父会话上）。
#[test]
pub(crate) fn approved_plan_spawns_a_sub_session_per_ready_node() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let after = core
        .collab_continue(&sid, CollabStep::Decide, "同意开工")
        .unwrap();

    let started: Vec<(String, String, String)> = after
        .iter()
        .filter_map(|e| match e {
            SessionEvent::NodeStarted {
                node,
                sid,
                assignee,
            } => Some((node.clone(), sid.clone(), assignee.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(started.len(), 1, "一个就绪节点该建一个子会话：{:?}", after);
    let (node, child, assignee) = &started[0];
    assert_eq!(node, "n1-1", "序号由阶段派生");
    assert_eq!(assignee, "a");
    assert_eq!(
        child,
        &format!("{}--a", sid),
        "一个 agent 一个会话（不是一节点一会话）"
    );

    // 子会话是**普通单 agent 会话**：meta 记着编排者与节点，沙箱锚在父会话上。
    let (cmeta, _) = core.history_open(child).unwrap();
    assert_eq!(cmeta.parent.as_deref(), Some(sid.as_str()));
    assert!(
        cmeta.node.is_none(),
        "节点不再记在会话 meta 里：一个 agent 一个会话，哪个节点正跑在它里面由链的 sub_session 认"
    );
    assert_eq!(cmeta.mode, "single");
    assert_eq!(cmeta.work(), sid.as_str(), "沙箱锚在父会话上（共用工作区）");
    assert_eq!(cmeta.agents.len(), 1, "子会话只有一个席位");
    assert_eq!(cmeta.agents[0].name, "a");
}

/// 同一个 agent 的多个节点**串行**（一个会话一次只能跑一轮）；不同 agent 照旧并发。
#[test]
pub(crate) fn same_agent_nodes_serialize_but_different_agents_run_together() {
    let mut member = BTreeMap::new();
    for who in ["a", "b"] {
        member.insert(
            who.to_string(),
            vec![
                "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
            ],
        );
    }
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(
            member,
            vec![
                // 三个节点都没有依赖：n1/n2 都归 a（该串行），n3 归 b（该和 n1 一起开工）。
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"一\",\"objective\":\"做一\",\"assignee\":\"a\",\"deps\":[]},{\"id\":\"n2\",\"title\":\"二\",\"objective\":\"做二\",\"assignee\":\"a\",\"deps\":[]},{\"id\":\"n3\",\"title\":\"三\",\"objective\":\"做三\",\"assignee\":\"b\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"},{\"node\":\"n1-2\",\"ok\":true,\"note\":\"够用\"},{\"node\":\"n1-3\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a", "b"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let evs = core
        .collab_continue(&sid, CollabStep::Decide, "同意开工")
        .unwrap();
    let order: Vec<String> = evs
        .iter()
        .filter_map(|e| match e {
            SessionEvent::NodeStarted { node, .. } => Some(node.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec!["n1-1", "n1-3", "n1-2"],
        "同一 agent 的 n2 必须等 n1 跑完；不同 agent 的 n3 与 n1 一起开工：{:?}",
        evs
    );
}

/// 返工定向的**字段契约**：fail 必须指名节点 id（核心只把那些节点退回待办重派）；
/// 表里没有的 id / 漏填 = 这次判定用不了 → 核心据此要求重填，不静默丢掉一条判定。
#[test]
pub(crate) fn checklist_rework_is_a_validated_node_id() {
    let known = vec!["n1-1".to_string(), "n2-1".to_string()];
    let item = |status: &str, rework: Option<&str>| {
        crate::capabilities::collab::domain::engine::CheckItem {
            item: "方案条目".to_string(),
            status: status.to_string(),
            evidence: None,
            reason: Some("还差一步".to_string()),
            rework: rework.map(|r| r.to_string()),
        }
    };
    let mut exec = crate::capabilities::collab::domain::engine::Execution::new();
    exec.items = vec![item("fail", Some("n2-1")), item("pass", None)];
    assert_eq!(exec.rework_targets(), vec!["n2-1".to_string()]);
    assert!(
        exec.rework_problems(&known).is_empty(),
        "合法 id 不该被判无效"
    );
    assert!(!exec.all_pass());
    exec.items = vec![item("fail", None)];
    assert_eq!(exec.rework_problems(&known).len(), 1, "漏填要重填");
    assert!(exec.rework_targets().is_empty(), "没指名就不退任何节点");
    exec.items = vec![item("fail", Some("n9"))];
    assert_eq!(
        exec.rework_problems(&known).len(),
        1,
        "表里没有的 id 要重填"
    );
    exec.items = vec![item("pass", None)];
    assert!(exec.rework_problems(&known).is_empty());
    assert!(exec.all_pass());
}

/// 阶段验收的 id 填错**不改系统状态**：核心被要求重填，重填对了才照常推进（不设次数上限）。
#[test]
pub(crate) fn stage_review_refills_until_the_ids_are_valid() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"把事做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                // 第一次把 id 写错（不在表里）→ 核心必须重填，而不是把判定丢掉或假装过了。
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n9\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let evs = core
        .collab_continue(&sid, CollabStep::Decide, "同意开工")
        .unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("重填"))),
        "id 不在表里该要求核心重填：{:?}",
        evs.iter()
            .filter_map(|e| match e {
                SessionEvent::Notice(n) => Some(n.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "重填对了就照常推进并交付"
    );
}

/// 总验收没过 → **核心指名要返工的节点**（rework 字段）：只退这些、暂停交用户；
/// 点「继续」只重派它们，重验通过才交付（不整条链重来）。
#[test]
pub(crate) fn total_review_rework_names_the_nodes_and_only_they_are_redispatched() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let script = |items: &str| {
        format!(
            "{{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{{\"items\":{}}}}}",
            items
        )
    };
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"把事做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                // 总验收：没过，并**指名**要返工的节点（不写人名）。
                script("[{\"item\":\"方案条目\",\"status\":\"fail\",\"reason\":\"还差依据\",\"rework\":\"n1-1\"}]"),
                // 点「继续」之后：阶段重验 + 总验收通过 → 交付。
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                script("[{\"item\":\"方案条目\",\"status\":\"pass\",\"evidence\":\"回报\"}]"),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let first = core
        .collab_continue(&sid, CollabStep::Decide, "同意开工")
        .unwrap();
    match core.collab_pending(&sid).unwrap() {
        Some(Pending::NodeBlocked { nodes }) => {
            assert_eq!(nodes, vec!["n1-1".to_string()], "只退核心指名的那个节点")
        }
        other => panic!("总验收没过该暂停并指名要返工的节点：{:?}", other),
    }
    assert!(
        !first
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { .. })),
        "没过就不交付"
    );
    assert!(
        first
            .iter()
            .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("只重派这些"))),
        "如实说明只重派指名的那些"
    );
    // 用户看到的顺序：**完成**先报（节点提交那一刻），再是**返工提示**、最后那三句汇总。
    let done_at = first
        .iter()
        .position(|e| matches!(e, SessionEvent::Report { id, .. } if id == "n1-1"));
    let rework_at = first.iter().position(
        |e| matches!(e, SessionEvent::Notice(n) if n.contains("[返工]") && n.contains("n1-1")),
    );
    let tail_at = first
        .iter()
        .position(|e| matches!(e, SessionEvent::Notice(n) if n.contains("只重派这些")));
    assert!(
        done_at.is_some() && rework_at.is_some(),
        "要有「节点完成」与「返工提示」：{:?}",
        first
    );
    assert!(
        done_at < rework_at && rework_at < tail_at,
        "顺序该是：节点完成 → 返工提示 → 汇总（{:?}/{:?}/{:?}）",
        done_at,
        rework_at,
        tail_at
    );
    // **唤醒不能替用户点「继续」**：单纯再推一步（子会话完成叫醒走的就是这条）不该重派任何节点，
    // 否则"暂停等你定"形同虚设。
    let wake = core.collab_advance(&sid).unwrap();
    assert!(
        !wake
            .iter()
            .any(|e| matches!(e, SessionEvent::NodeStarted { .. })),
        "挂着等用户时，唤醒不能自己重派：{:?}",
        wake
    );
    // **核心在干什么要落档**（它没有 agent 会话，所以它的行由协作会话接住并落盘）。
    let (_, hist) = core.history_open("w").unwrap();
    let rows = replay_lines(&hist);
    for want in [
        "[核心:plan]",
        "[核心:verdict]",
        "[核心:node_verdict]",
        "[核心:checklist]",
    ] {
        assert!(
            rows.iter().any(|r| r.starts_with(want)),
            "核心的操作该有落档的行 {}：{:?}",
            want,
            rows.iter()
                .filter(|r| r.starts_with("[核心"))
                .collect::<Vec<_>>()
        );
    }
    let second = core.collab_resume(&sid).unwrap();
    let redispatch: Vec<&String> = second
        .iter()
        .filter_map(|e| match e {
            SessionEvent::NodeStarted { node, .. } => Some(node),
            _ => None,
        })
        .collect();
    assert!(
        !redispatch.is_empty() && redispatch.iter().all(|n| *n == "n1-1"),
        "只重派指名的节点：{:?}",
        redispatch
    );
    assert!(
        second
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "重派并验过之后才交付：{:?}",
        second
    );
}

/// 节点验收没过 → **暂停并交用户**；点「继续」重派该节点，验过才交付。
#[test]
pub(crate) fn failed_node_acceptance_pauses_then_continue_redispatches() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                // 第一次节点验收：没过 → 该暂停等用户。
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":false,\"note\":\"还差依据\"}]}}".to_string(),
                // 「继续」之后重派并再验：这次过。
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();

    let first = core
        .collab_continue(&sid, CollabStep::Decide, "同意开工")
        .unwrap();
    assert!(
        matches!(
            core.collab_pending(&sid),
            Ok(Some(Pending::NodeBlocked { .. }))
        ),
        "节点没过该暂停并交用户：{:?}",
        first
    );
    assert!(
        !first
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { .. })),
        "没过就不该交付：{:?}",
        first
    );

    // 点「继续」：把没过的节点退回待办并重派，再验通过 → 交付。
    let second = core.collab_resume(&sid).unwrap();
    assert!(
        second
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "重派后验过就该交付：{:?}",
        second
    );
}

/// 审查关卡：整理完**不自动开工**——停在待审，点「同意」才推进（见 docs/architecture/task-chain.md）。
#[test]
pub(crate) fn collab_pauses_for_plan_review_until_the_user_approves() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意方案\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;

    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::PlanReview { .. })),
        "整理完该停在**待审**：{:?}",
        events
    );
    assert!(
        !events.iter().any(|e| matches!(
            e,
            SessionEvent::Report { .. } | SessionEvent::Delivery { .. }
        )),
        "没过审不该开工：{:?}",
        events
    );
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::PlanReview))),
        "待审要挂起等用户"
    );
    // 链随方案一起交给用户审查：节点、负责人、目标都在（审查关卡看的就是这张图）。
    let reviewed = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::PlanReview { chain, .. } => Some(chain.clone()),
            _ => None,
        })
        .expect("待审事件要带链");
    assert_eq!(reviewed.nodes.len(), 1, "链里该有一个节点：{:?}", reviewed);
    assert_eq!(reviewed.nodes[0].assignee, "a");
    assert_eq!(reviewed.nodes[0].objective, "把 X 做完");

    // 点「同意」之后才推进：执行回报与交付都该出现。
    let after = core
        .collab_continue(&sid, CollabStep::Decide, "同意开工")
        .unwrap();
    assert!(
        after
            .iter()
            .any(|e| matches!(e, SessionEvent::Report { .. })),
        "同意后该开工：{:?}",
        after
    );
    assert!(
        after
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { .. })),
        "同意后该走到交付：{:?}",
        after
    );
}

#[test]
pub(crate) fn core_collab_demo_runs_full_five_stages() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意方案\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(
            core.collab_continue(&sid, CollabStep::Decide, "同意开工")
                .unwrap(),
        );
        e
    };
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Plan(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
    // 终结会话已被中心回收（再查询挂起状态应报无此会话）。
    assert!(core.collab_pending(&sid).is_err());
}

#[test]
pub(crate) fn core_collab_delegated_slate_flow() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        // 代拟（组装一个 agent）→ 整理 → 验收。
        "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"a\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string(),
    ]));
    let opened = core
        .create_work(collab_work("w", &[], true, "做个东西"))
        .unwrap();
    let sid = opened.sid;
    let ev = opened.facts;
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmSlate))
    ));
    assert!(ev.iter().any(
        |e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.speaker == "代拟"))
    ));
    let _ = core
        .collab_continue(&sid, CollabStep::ConfirmSlate, "yes")
        .unwrap();
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(
            core.collab_continue(&sid, CollabStep::Decide, "同意开工")
                .unwrap(),
        );
        e
    };
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
}

#[test]
pub(crate) fn core_collab_slate_rejects_invalid_picks() {
    // 不存在的模块 / 不存在的模型 / 不在登记处的复用项：整条拒收，合法的留下。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"鬼\",\"modules\":[\"ghost\"],\"model\":\"m\",\"why\":\"模块不存在\"},{\"agent\":\"幽灵\",\"why\":\"不在登记处\"},{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"nope\",\"why\":\"模型不存在\"},{\"name\":\"乙\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案\"}".to_string(),
        "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"x\",\"status\":\"pass\"}]}}".to_string(),
    ]));
    let ev = core
        .create_work(collab_work("w", &[], true, "任务"))
        .unwrap()
        .facts;
    for want in ["ghost 不存在", "幽灵 不在登记处", "nope 不存在"] {
        assert!(
            ev.iter()
                .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains(want))),
            "应如实拒收：{}",
            want
        );
    }
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("乙〈a〉→ m")))));
}

#[test]
pub(crate) fn collab_delegated_roster_written_back_and_rebuilt_from_meta() {
    // 发言席是 agent：脚本按 agent 名 "调研员" 回放（不再按模块 id）。
    let mut member = BTreeMap::new();
    member.insert(
        "调研员".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        "{\"type\":\"tool\",\"name\":\"slate\",\"args\":{\"picks\":[{\"name\":\"调研员\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}}".to_string(),
        // 负责人必须是**名单里真实存在的席位**（代拟出来的叫"调研员"）——否则链的自洽门禁会如实挡下。
        "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"调研员\",\"deps\":[]}]}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
        "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string(),
    ]));
    let sid = core
        .create_work(collab_work("w", &[], true, "做个东西"))
        .unwrap()
        .sid;
    assert!(
        core.history_open("w").unwrap().0.agents.is_empty(),
        "确认名单之前不落档（名单只活在内存里）"
    );
    // CLI 的确认门要能把这份表单逐行读出来。
    let slate = core.collab_slate(&sid).unwrap();
    assert_eq!(slate.len(), 1);
    assert_eq!(slate[0].name, "调研员");
    assert!(slate[0].transient, "组装项如实标记为临时 agent");
    assert_eq!(slate[0].model.as_deref(), Some("m"));

    core.collab_continue(&sid, CollabStep::ConfirmSlate, "yes")
        .unwrap();
    // 确认后名单写回 meta（重启/回档后的权威来源）。
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研员");
    assert_eq!(meta.agents[0].modules, vec!["a".to_string()]);
    assert_eq!(meta.agents[0].model.as_deref(), Some("m"));
    assert_eq!(meta.modules, vec!["a".to_string()]);

    // 回档（保留需求行）= 按 meta.agents 重建会话（协作走「按转录重建」这条路），再跑到交付。
    core.rewind(&sid, 1).unwrap();
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))),
        "重建后仍等确认开始"
    );
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(
            core.collab_continue(&sid, CollabStep::Decide, "同意开工")
                .unwrap(),
        );
        e
    };
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "重建出来的名单要能一路跑完：{:?}",
        events
    );
}

// ---------- 平衡提取器 ----------

/// 讨论回合也要**逐片外送**（此前 Chunk 只用来当中止信号、内容全丢，界面整回合不动）。
#[test]
pub(crate) fn discussion_turn_streams_deltas_and_never_leaks_the_envelope() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut chat = super::RecordingChat {
        // 正文在信封**之前**：正文可以逐片流出去，信封本身绝不能。
        inner: scripted(vec![
            "我说两句。{\"type\":\"say\",\"text\":\"我说两句\"}".into()
        ]),
        seen: Arc::clone(&seen),
    };
    let mut events: Vec<crate::capabilities::session::api::SessionEvent> = Vec::new();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prompts = test_prompts();
    let _ = crate::capabilities::collab::domain::engine::Discussion::turn_with(
        &*test_tools_svc(),
        "discussant",
        &cancel,
        crate::capabilities::llm::api::CompleteOpts::plain(true), // 开流式
        "a",
        "（测试）身份",
        &[],
        &mut chat,
        None,
        vec![crate::capabilities::llm::api::Msg::user("说说")],
        &prompts.tools(),
        &mut |e| events.push(e),
    )
    .expect("跑一个回合");
    let kinds: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            crate::capabilities::session::api::SessionEvent::Delta { kind, .. } => {
                Some(kind.clone())
            }
            _ => None,
        })
        .collect();
    assert!(
        kinds.iter().any(|k| k == "start"),
        "要有起始片：{:?}",
        kinds
    );
    assert!(kinds.iter().any(|k| k == "text"), "要有正文片：{:?}", kinds);
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            crate::capabilities::session::api::SessionEvent::Delta { kind, text, .. }
                if kind == "text" =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert!(text.contains("我说两句。"), "正文该逐片流出去：{}", text);
    assert!(
        !text.contains("\"type\"") && !text.contains('{'),
        "信封不能当正文流出去：{}",
        text
    );
}

#[test]
pub(crate) fn core_collab_tool_modules_run_in_execution() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "  1 | 内容".into(),
        ok: true,
    });
    let mut member = BTreeMap::new();
    // 脚本按**通道**各自一份（子会话拿的是新的一份）：所以第一项要同时能应付两条路——
    // 讨论开场收下它（tool 动词在讨论里只落一行），节点执行则真的跑这个工具。
    // 一个 agent 一个会话：讨论与**节点执行**共用同一条通道，所以脚本按"整场经历"排。
    // 开场同意（收敛）→ 整理 → 同意方案 → 节点执行时真的跑那个工具。
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
            TOOL_CALL.to_string(),
            "{\"type\":\"say\",\"text\":\"执行完毕\"}".to_string(),
        ],
    );
    let mut mod_a = module_of("a");
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), decl("python tools/grep.py"));
    mod_a.manifest.tools = manifest_tools;
    // 核心脚本：整理（方案）→ 验收（全过）。
    let mut core = core_with_runner(
        vec![mod_a],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：查证后回报\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".into(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".into(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".into(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"查证\",\"status\":\"pass\"}]}}".into(),
            ],
        ),
        Arc::clone(&runner),
    );
    let opened = core
        .create_work(collab_work("w", &["a"], false, "任务"))
        .unwrap();
    let sid = opened.sid;
    let mut events = opened.facts;
    events.extend(core.collab_continue(&sid, CollabStep::Begin, "").unwrap());
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    events.extend(
        core.collab_continue(&sid, CollabStep::Decide, "同意开工")
            .unwrap(),
    );
    // 工具在**节点自己的子会话**里跑：那条 tool 转录行落在子会话的转录上（单 agent 行格式）。
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Transcript(ls)
            if ls.iter().any(|l| l.tool.is_some() && l.line.contains("成功")))),
        "节点执行里的工具调用应发 tool 转录行（子会话自己的转录）",
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "验收应通过"
    );
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}
// ---------- 运行包：契约、包库、诊断、执行计划 ----------

/// 核心操作必须走**工具调用**：正文里手写 JSON 不再被接受（真机上它既无 schema 校验也不进工具台账）。
#[test]
pub(crate) fn core_operations_require_a_tool_call_not_body_json() {
    let systools = test_systools();
    // 角色表把核心操作发给对应的核心身份（越权校验与工具面的判据都是它）。
    for (role, tool) in [
        ("planner", "plan"),
        ("planner", "slate"),
        ("planner", "suggest"),
        ("orchestrator", "node_verdict"),
        ("orchestrator", "checklist"),
    ] {
        assert!(
            systools.role_face(role).0.iter().any(|t| t == tool),
            "{} 该拿到 {} 工具",
            role,
            tool
        );
    }
    // 讨论席与执行席不拿核心操作（越权会被如实拒绝）。
    let face = |role: &str| systools.role_face(role).0;
    assert!(!face("discussant").iter().any(|t| t == "plan"));
    assert!(!face("executor").iter().any(|t| t == "checklist"));
    // 谁能用"自己模块的工具"也由角色表说了算：只有干活的那一席发（讨论席列出来等于请它去撞墙）。
    assert!(
        systools.allows_module_tools("executor"),
        "执行席要能用自己模块的工具"
    );
    assert!(
        !systools.allows_module_tools("discussant"),
        "讨论席不发模块工具"
    );
    assert!(!systools.allows_module_tools("planner"));
    // 载荷是**数组**参数（嵌套结构），不是标量。
    let schema = systools.tools.get("plan").expect("plan 该在工具总表里");
    assert_eq!(
        schema.params.as_ref().expect("有参数")["nodes"].ty,
        crate::capabilities::workspace::api::ParamType::Array,
        "任务链节点表是数组载荷"
    );
}

/// **核心操作也要能先核实**：模型先发只读核实（read），核心执行并把结果回灌，
/// 然后再要那一次核心操作调用（plan）。此前是单次调用——模型一想核实就被判"没有调用 plan"，
/// 整步中断（真机上核心就是这么卡在多轮 `[中断] 没有调用 node_verdict` 上的）。
#[test]
pub(crate) fn core_operation_runs_readonly_verification_before_the_op() {
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["demo", "work", "note.txt"]);
    io.seed(&["demo", "work", "note.txt"], "现场：一切正常\n");
    let sb = test_sandbox("核心", &[]);
    let mut verify = MemberTools {
        mode: crate::capabilities::llm::api::ToolMode::Envelope,
        modules: BTreeMap::new(),
        observations: crate::capabilities::tools::api::Observations::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::log::NoopLog),
        tools: test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
        sandbox: sb.clone(),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::capabilities::tools::api::FenceSpec::from_sandbox(&sb, false),
        reply_seq: 0,
        allowed: vec!["read".to_string(), "plan".to_string()],
        with_modules: false,
        notes: crate::capabilities::tools::api::ToolNotes::default(),
    };
    // 第一轮：先核实（read）；第二轮：交出 plan。
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut chat = super::RecordingChat {
        inner: scripted(vec![
            format!(
                "{{\"type\":\"tool\",\"name\":\"read\",\"args\":{{\"path\":\"{}\"}}}}",
                note
            ),
            "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[]}}"
                .to_string(),
        ]),
        seen: Arc::clone(&seen),
    };
    let out = crate::capabilities::collab::domain::engine::core_operation(
        &*test_tools_svc(),
        "planner",
        "plan",
        crate::capabilities::llm::api::ToolMode::Envelope,
        &mut chat,
        &[crate::capabilities::llm::api::Msg::user("出方案")],
        crate::capabilities::llm::api::CompleteOpts::plain(false),
        &mut |_| true,
        Some(&mut verify),
        &mut |_e: crate::capabilities::session::api::SessionEvent| {},
    )
    .expect("核实之后要能交出方案");
    assert_eq!(out["plan"], "方案");
    // 核实那次真的执行并回灌了：第二轮请求的消息里带着它的结果。
    let calls = seen.lock().expect("锁");
    assert!(
        calls.len() >= 2,
        "至少两次模型调用（核实 + 交出）：{}",
        calls.len()
    );
    assert!(
        calls[1].iter().any(|m| m.contains("现场：一切正常")),
        "第二轮请求要带上核实结果：{:?}",
        calls[1]
    );
}

/// 正文里手写 JSON 不再被当成核心操作：**如实报错**，不把原文糊成方案。
#[test]
pub(crate) fn body_json_is_not_a_core_operation() {
    let mut chat = scripted(vec![
        "{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}"
            .to_string(),
    ]);
    let out = crate::capabilities::collab::domain::engine::core_operation(
        &*test_tools_svc(),
        "planner",
        "plan",
        crate::capabilities::llm::api::ToolMode::Envelope,
        chat.as_mut(),
        &[crate::capabilities::llm::api::Msg::user("出方案")],
        crate::capabilities::llm::api::CompleteOpts::plain(false),
        &mut |_| true,
        None,
        &mut |_e: crate::capabilities::session::api::SessionEvent| {},
    );
    let err = out.expect_err("正文 JSON 不是工具调用，该如实报错");
    assert!(err.contains("没有调用 plan"), "{}", err);
}

/// 回报走**工具调用**（不是正文 JSON）：执行席拿得到它，调用它不碰文件、回执带出回报内容。
#[test]
pub(crate) fn executor_reports_through_a_tool_call() {
    // 角色表把 report 发给执行席（越权校验的判据就是它）。
    let face = |role: &str| test_systools().role_face(role).0;
    assert!(
        face("executor").iter().any(|t| t == "submit_report"),
        "执行席该拿到回报工具"
    );
    assert!(
        !face("discussant").iter().any(|t| t == "submit_report"),
        "讨论席不该拿到回报工具"
    );
    // 它不是文件域工具：没有 path 也照跑，回执把三个字段带出来。
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    let mut obs = crate::capabilities::tools::api::Observations::default();
    let out = crate::capabilities::tools::domain::systool::execute(
        &sb,
        &test_systools().tools,
        &io,
        &mut obs,
        "submit_report",
        "{\"summary\":\"抽了语料\",\"changes\":\"work/corpus.jsonl\",\"open\":\"\"}",
    );
    assert!(out.ok, "回报该成功：{:?}", out.output);
    assert!(
        out.output.contains("抽了语料"),
        "回执要带出 summary: {}",
        out.output
    );
    assert!(
        out.output.contains("work/corpus.jsonl"),
        "回执要带出 changes: {}",
        out.output
    );
}

/// 返工必须带上**上次没通过的原因**：否则 agent 只能把同一件事原样再做一遍。
#[test]
pub(crate) fn rework_prompt_carries_the_acceptance_note() {
    // 验收没过时，渲染出的提示词要含上次的原因；首轮（没有结论）不出现返工段。
    let p = test_prompts();
    let note = "报告里缺了坏链检查";
    let rework = format!(
        "\n== 上次没通过的原因 ==\n{}\n这次请针对上面的原因返工。\n",
        note
    );
    let out = p.render(
        Segment::ExecuteUser,
        &[("tasks", "把语料抽出来".to_string()), ("rework", rework)],
    );
    assert!(out.contains(note), "返工提示词要带上次的原因：{out}");
    let first = p.render(
        Segment::ExecuteUser,
        &[("tasks", "x".to_string()), ("rework", String::new())],
    );
    assert!(!first.contains("上次没通过"), "首轮不该出现返工段：{first}");
}
