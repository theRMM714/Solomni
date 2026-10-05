//! 整理、审查关卡、节点验收与返工：清单校验与重派
use super::super::builders::*;
use super::super::prelude::*;
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
    let mut exec = crate::capabilities::collab::service::synthesis::Execution::new();
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
    let mut exec2 = crate::capabilities::collab::service::synthesis::Execution::new();
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
    let mut exec = crate::capabilities::collab::service::synthesis::Execution::new();
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

// ---------- Conductor 门面：会话中心（内存组合根） ----------

#[test]
pub(crate) fn mode_vocabulary_is_single_or_collab_only() {
    // web 与 conductor 的形态词汇只有 single / collab；旧的 direct / compose 一概不认（GREEN FIELD，无兼容）。
    use crate::presentation::web::parse_mode;
    assert!(matches!(parse_mode("single"), Ok(WorkMode::Single)));
    assert!(matches!(parse_mode("collab"), Ok(WorkMode::Collab)));
    // 第三人形态：代理（没有名单，选它就是授予全权）。
    assert!(matches!(parse_mode("proxy"), Ok(WorkMode::Proxy)));
    assert!(parse_mode("nope").is_err());
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
            permissions: Default::default(),
        }],
        exec: ExecSpec::default(),
        parent: None,
        node: None,
        delegation: None,
        run: RunState::Active,
    })
    .unwrap();
    // 内存里没有这个会话 → 走 rebuild_session，对未知形态如实报错。
    let err = core
        .rewind(
            "旧会话",
            crate::capabilities::conductor::api::RewindTarget::Delete(0),
        )
        .unwrap_err();
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
    assert_eq!(
        core.work_root(child).expect("工作根"),
        sid,
        "沙箱锚在顶层工作上（整棵树共用一个 work/）"
    );
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
        crate::capabilities::collab::service::synthesis::CheckItem {
            item: "方案条目".to_string(),
            status: status.to_string(),
            evidence: None,
            reason: Some("还差一步".to_string()),
            rework: rework.map(|r| r.to_string()),
        }
    };
    let mut exec = crate::capabilities::collab::service::synthesis::Execution::new();
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

/// 审查关卡：整理完**不自动开工**——停在待审，点「同意」才推进（见 docs/collab/task-chain.md）。
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
