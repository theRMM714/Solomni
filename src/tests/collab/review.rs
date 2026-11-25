//! 目的：钉整理、审查关卡、节点验收与返工的清单校验契约。
//! 管：过审按就绪节点派子会话、同 agent 串行、返工 id 校验、阶段验收重填、总验收指名返工、失败暂停后继续、计划审批暂停。
//! 不管：讨论泵（discussion.rs）与状态派生（state.rs）。
//! 联动：src/units/collab/、taskchain、docs/collab/task-chain.md、tests/doubles。

use super::super::builders::*;
use super::super::prelude::*;
use crate::units::conversation::session::api::{MEETING, PROXY, SINGLE};
// ---------- Core 门面：会话中心（内存组合根） ----------
#[test]
pub(crate) fn mode_vocabulary_is_single_or_collab_only() {
    // 呈现层只把会话类型原样往下传：在册与否由工作能力按会话类型表判（见 detail/profiles.rs）。
    use crate::presentation::web::parse_kind;
    assert_eq!(parse_kind("single").unwrap(), SINGLE);
    assert_eq!(parse_kind(" collab ").unwrap(), MEETING);
    assert_eq!(parse_kind("proxy").unwrap(), PROXY);
    assert_eq!(
        parse_kind("自己拼的类型").unwrap(),
        "自己拼的类型",
        "临时类型名照传"
    );
    for bad in ["", "   "] {
        let e = parse_kind(bad).unwrap_err();
        assert!(e.contains("kind 不能为空"), "web 必须 400 并说明：{}", e);
    }
}

/// 目的：方案过审后：就绪节点各建一个**子会话**（普通单 agent 会话，沙箱锚在父会话上）。
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
                // 讨论段收敛后，核心先写一份总结——它就是执行段的需求。
                                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(execute_work("w", &["a"], "做个东西"))
        .unwrap()
        .sid;
    advance(&mut core, &sid);
    let after = answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap();

    let started: Vec<(String, String, String)> = after
        .iter()
        .filter_map(|e| match CollabEvent::of(e) {
            Some(CollabEvent::NodeStarted {
                node,
                sid,
                assignee,
            }) => Some((node, sid, assignee)),
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
    let (cmeta, _) = core.lifecycle.history_open(child).unwrap();
    assert_eq!(cmeta.parent.as_deref(), Some(sid.as_str()));
    assert!(
        cmeta.node.is_none(),
        "节点不再记在会话 meta 里：一个 agent 一个会话，哪个节点正跑在它里面由链的 sub_session 认"
    );
    assert_eq!(
        cmeta.spec.kind, "",
        "节点子会话不套预设类型：身份由流程的成员席给"
    );
    assert_eq!(
        crate::units::conversation::session::api::work_root(core.history_handle().as_ref(), child)
            .expect("工作根"),
        sid,
        "沙箱锚在顶层工作上（整棵树共用一个 work/）"
    );
    assert_eq!(cmeta.agents.len(), 1, "子会话只有一个席位");
    assert_eq!(cmeta.agents[0].name, "a");
}

/// 目的：协作的裁决事实（方案 / 验收清单 / 交付）落成**带 id 的转录行**，不是只推一条事件：
///   刷新与回档后照样读得到；行种类是 `notice`（只给人看、不进模型上下文），身份在说话人里。
///   管：三种事实各落一行、方案行排在闸门卡之前、行 id 互不相同。
#[test]
pub(crate) fn collab_facts_land_as_replayable_transcript_rows() {
    // 从事件里读转录行（走线上同一条 to_json，再整行反序列化：判据读结构化字段）。
    fn rows(evs: &[SessionEvent]) -> Vec<crate::units::transcript::api::LineView> {
        let mut out = Vec::new();
        for ev in evs {
            let v = ev.to_json();
            let Some(ls) = v.get("lines").and_then(|l| l.as_array()) else {
                continue;
            };
            for l in ls {
                if let Ok(line) =
                    serde_json::from_value::<crate::units::transcript::api::LineView>(l.clone())
                {
                    out.push(line);
                }
            }
        }
        out
    }

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
        .create_work(execute_work("w", &["a"], "做个东西"))
        .unwrap()
        .sid;
    let mut evs = advance(&mut core, &sid);

    // 方案行：正文就是核心整理出来的那份方案，身份是"核心整理"。
    let plan_row = rows(&evs)
        .into_iter()
        .find(|l| l.speaker == "核心整理")
        .expect("方案要落成转录行");
    assert_eq!(plan_row.kind, "notice", "只给人看、不进模型上下文");
    assert!(!plan_row.system, "不能变成上下文里的系统消息");
    assert!(
        plan_row.line.contains("方案：A 做 X"),
        "行里是方案正文：{}",
        plan_row.line
    );
    // 顺序：用户先读到方案，再看到"要不要现在开工"的闸门卡。
    let mut plan_at = usize::MAX;
    let mut gate_at = usize::MAX;
    for (i, ev) in evs.iter().enumerate() {
        if matches!(CollabEvent::of(ev), Some(CollabEvent::PlanReview { .. })) {
            gate_at = i;
        }
        if ev
            .to_json()
            .get("lines")
            .and_then(|l| l.as_array())
            .is_some_and(|ls| {
                ls.iter().any(|l| {
                    serde_json::from_value::<crate::units::transcript::api::LineView>(l.clone())
                        .is_ok_and(|x| x.speaker == "核心整理")
                })
            })
        {
            plan_at = plan_at.min(i);
        }
    }
    assert!(
        plan_at < gate_at,
        "方案行要在闸门卡之前：{:?}",
        shown_texts(&evs)
    );

    // 过审 → 派节点 → 阶段验收 → 总验收 → 交付：清单与交付各落一行。
    evs.extend(answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap());
    let all = rows(&evs);
    let review_row = all
        .iter()
        .find(|l| l.speaker == "验收清单")
        .expect("总验收清单要落成转录行");
    assert_eq!(review_row.kind, "notice");
    assert!(
        review_row.line.contains("**做 X** · 通过"),
        "清单一行一条、结论两档：{}",
        review_row.line
    );
    let delivery_row = all
        .iter()
        .find(|l| l.speaker == "交付")
        .expect("交付结论要落成转录行");
    assert_eq!(delivery_row.kind, "notice");
    assert!(
        delivery_row.line.contains("交付用户"),
        "全过就如实说交付：{}",
        delivery_row.line
    );
    // 落盘可回放：刷新/回档后从历史里读得到同一批行（不是只活在这条事件流里）。
    let (_, hist) = core.lifecycle.history_open(&sid).unwrap();
    let persisted: Vec<crate::units::transcript::api::LineView> = hist
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flat_map(|ls| ls.to_vec())
        .filter_map(|l| serde_json::from_value::<crate::units::transcript::api::LineView>(l).ok())
        .collect();
    for who in ["核心整理", "验收清单", "交付"] {
        assert!(
            persisted
                .iter()
                .any(|l| l.speaker == who && l.kind == "notice" && l.id > 0),
            "「{}」这一行要落档（带 id）：{:?}",
            who,
            persisted.iter().map(|l| l.render()).collect::<Vec<_>>()
        );
    }
    // 行 id 互不相同（回档按 id 截断，撞号会切错地方）：三条事实各占一个号。
    let mut ids: Vec<u64> = persisted
        .iter()
        .filter(|l| ["核心整理", "验收清单", "交付"].contains(&l.speaker.as_str()))
        .map(|l| l.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        3,
        "三条事实行各占一个不同的 id：{:?}",
        persisted
            .iter()
            .map(|l| format!("{}#{}:{}", l.speaker, l.id, l.kind))
            .collect::<Vec<_>>()
    );
}

/// 目的：同一个 agent 的多个节点**串行**（一个会话一次只能跑一轮）；不同 agent 照旧并发。
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
                // 讨论段收敛后，核心先写一份总结——它就是执行段的需求。
                                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"一\",\"objective\":\"做一\",\"assignee\":\"a\",\"deps\":[]},{\"id\":\"n2\",\"title\":\"二\",\"objective\":\"做二\",\"assignee\":\"a\",\"deps\":[]},{\"id\":\"n3\",\"title\":\"三\",\"objective\":\"做三\",\"assignee\":\"b\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"},{\"node\":\"n1-2\",\"ok\":true,\"note\":\"够用\"},{\"node\":\"n1-3\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(execute_work("w", &["a", "b"], "做个东西"))
        .unwrap()
        .sid;
    advance(&mut core, &sid);
    let evs = answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap();
    let order: Vec<String> = evs
        .iter()
        .filter_map(|e| match CollabEvent::of(e) {
            Some(CollabEvent::NodeStarted { node, .. }) => Some(node),
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

/// 目的：阶段验收的 id 填错**不改系统状态**：核心被要求重填，重填对了才照常推进（不设次数上限）。
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
                // 讨论段收敛后，核心先写一份总结——它就是执行段的需求。
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
        .create_work(execute_work("w", &["a"], "做个东西"))
        .unwrap()
        .sid;
    advance(&mut core, &sid);
    let evs = answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap();
    assert!(
        shown_contains(&evs, "重填"),
        "id 不在表里该要求核心重填：{:?}",
        shown_texts(&evs)
    );
    assert!(
        evs.iter().any(|e| matches!(
            CollabEvent::of(e),
            Some(CollabEvent::Delivery { ok: true, .. })
        )),
        "重填对了就照常推进并交付"
    );
}

/// 目的：总验收没过 → **核心指名要返工的节点**（rework 字段）：只退这些、暂停交用户；
///   点「继续」只重派它们，重验通过才交付（不整条链重来）。
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
                // 讨论段收敛后，核心先写一份总结——它就是执行段的需求。
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
        .create_work(execute_work("w", &["a"], "做个东西"))
        .unwrap()
        .sid;
    advance(&mut core, &sid);
    let first = answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap();
    match core.turn.collab_pending(&sid).unwrap() {
        Some(Pending::NodeBlocked { nodes }) => {
            assert_eq!(nodes, vec!["n1-1".to_string()], "只退核心指名的那个节点")
        }
        other => panic!("总验收没过该暂停并指名要返工的节点：{:?}", other),
    }
    assert!(
        !first
            .iter()
            .any(|e| matches!(CollabEvent::of(e), Some(CollabEvent::Delivery { .. }))),
        "没过就不交付"
    );
    assert!(
        shown_contains(&first, "只重派这些"),
        "如实说明只重派指名的那些"
    );
    // 用户看到的顺序：**完成**先报（节点提交那一刻），再是**返工提示**、最后那三句汇总。
    let done_at = first.iter().position(|e| {
        matches!(
            CollabEvent::of(e),
            Some(CollabEvent::Report { id, .. }) if id == "n1-1"
        )
    });
    let rework_at = shown_at(&first, "[返工] n1-1");
    let tail_at = shown_at(&first, "只重派这些");
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
    let wake = core
        .turn
        .collab_advance(&sid, core.registry.as_ref(), core.probe.as_ref())
        .unwrap();
    assert!(
        !wake
            .iter()
            .any(|e| matches!(CollabEvent::of(e), Some(CollabEvent::NodeStarted { .. }))),
        "挂着等用户时，唤醒不能自己重派：{:?}",
        wake
    );
    // **核心在干什么要落档**（它没有 agent 会话，所以它的行由协作会话接住并落盘）。
    let (_, hist) = core.lifecycle.history_open("w").unwrap();
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
    let second = core
        .turn
        .collab_resume(&sid, core.registry.as_ref(), core.probe.as_ref())
        .unwrap();
    let redispatch: Vec<String> = second
        .iter()
        .filter_map(|e| match CollabEvent::of(e) {
            Some(CollabEvent::NodeStarted { node, .. }) => Some(node),
            _ => None,
        })
        .collect();
    assert!(
        !redispatch.is_empty() && redispatch.iter().all(|n| *n == "n1-1"),
        "只重派指名的节点：{:?}",
        redispatch
    );
    assert!(
        second.iter().any(|e| matches!(
            CollabEvent::of(e),
            Some(CollabEvent::Delivery { ok: true, .. })
        )),
        "重派并验过之后才交付：{:?}",
        second
    );
}

/// 目的：节点验收没过 → **暂停并交用户**；点「继续」重派该节点，验过才交付。
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
                // 讨论段收敛后，核心先写一份总结——它就是执行段的需求。
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
        .create_work(execute_work("w", &["a"], "做个东西"))
        .unwrap()
        .sid;
    advance(&mut core, &sid);

    let first = answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap();
    assert!(
        matches!(
            core.turn.collab_pending(&sid),
            Ok(Some(Pending::NodeBlocked { .. }))
        ),
        "节点没过该暂停并交用户：{:?}",
        first
    );
    assert!(
        !first
            .iter()
            .any(|e| matches!(CollabEvent::of(e), Some(CollabEvent::Delivery { .. }))),
        "没过就不该交付：{:?}",
        first
    );

    // 点「继续」：把没过的节点退回待办并重派，再验通过 → 交付。
    let second = core
        .turn
        .collab_resume(&sid, core.registry.as_ref(), core.probe.as_ref())
        .unwrap();
    assert!(
        second.iter().any(|e| matches!(
            CollabEvent::of(e),
            Some(CollabEvent::Delivery { ok: true, .. })
        )),
        "重派后验过就该交付：{:?}",
        second
    );
}

/// 目的：审查关卡：整理完**不自动开工**——停在待审，点「同意」才推进（见 docs/collab/task-chain.md）。
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
                // 讨论段收敛后，核心先写一份总结——它就是执行段的需求。
                                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(execute_work("w", &["a"], "做个东西"))
        .unwrap()
        .sid;

    let events = advance(&mut core, &sid);
    assert!(
        events
            .iter()
            .any(|e| matches!(CollabEvent::of(e), Some(CollabEvent::PlanReview { .. }))),
        "整理完该停在**待审**：{:?}",
        events
    );
    assert!(
        !events.iter().any(|e| matches!(
            CollabEvent::of(e),
            Some(CollabEvent::Report { .. } | CollabEvent::Delivery { .. })
        )),
        "没过审不该开工：{:?}",
        events
    );
    assert!(
        matches!(
            core.turn.collab_pending(&sid),
            Ok(Some(Pending::PlanReview))
        ),
        "待审要挂起等用户"
    );
    // 链随方案一起交给用户审查：节点、负责人、目标都在（审查关卡看的就是这张图）。
    let reviewed = events
        .iter()
        .find_map(|e| match CollabEvent::of(e) {
            Some(CollabEvent::PlanReview { chain, .. }) => Some(chain),
            _ => None,
        })
        .expect("待审事件要带链");
    assert_eq!(reviewed.nodes.len(), 1, "链里该有一个节点：{:?}", reviewed);
    assert_eq!(reviewed.nodes[0].assignee, "a");
    assert_eq!(reviewed.nodes[0].objective, "把 X 做完");

    // 点「同意」之后才推进：执行回报与交付都该出现。
    let after = answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap();
    assert!(
        after
            .iter()
            .any(|e| matches!(CollabEvent::of(e), Some(CollabEvent::Report { .. }))),
        "同意后该开工：{:?}",
        after
    );
    assert!(
        after
            .iter()
            .any(|e| matches!(CollabEvent::of(e), Some(CollabEvent::Delivery { .. }))),
        "同意后该走到交付：{:?}",
        after
    );
}
