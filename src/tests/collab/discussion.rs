//! 讨论泵与发言规则：表态、退场、自裁、提醒与回合上限
use super::super::builders::*;
use super::super::prelude::*;
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
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
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
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
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
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Notice(n)
            if n.contains("[越权]") && n.contains("harvest.scan"))),
        "模块工具在讨论回合该被如实拒绝：{:?}",
        events
    );
}

/// 讨论席的一回合：**逐轮定稿**（核实行与发言行各是各的一条落档事件，不是攒到回合末一条），
/// 并且**实时落的行与回档重建出的对话同口径**——重启后子会话的上下文不歪
/// （回档按行截断历史，靠的就是这两边一致；见 docs/session/session-model.md 二之二、五）。
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
    answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
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
