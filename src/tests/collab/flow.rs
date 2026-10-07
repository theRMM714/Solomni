//! 端到端流程：整理派发、代拟确认、执行与验收、工具模块
use super::super::builders::*;
use super::super::prelude::*;
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
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap());
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
    let _ = answer_card(&mut core, &sid, OPT_SLATE_CONFIRM, "").unwrap();
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap());
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

    answer_card(&mut core, &sid, OPT_SLATE_CONFIRM, "").unwrap();
    // 确认后名单写回 meta（重启/回档后的权威来源）。
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研员");
    assert_eq!(meta.agents[0].modules, vec!["a".to_string()]);
    assert_eq!(meta.agents[0].model.as_deref(), Some("m"));
    assert_eq!(meta.modules, vec!["a".to_string()]);

    // 回档（保留需求行）= 按 meta.agents 重建会话（协作走「按转录重建」这条路），再跑到交付。
    core.rewind(
        &sid,
        crate::capabilities::conductor::api::RewindTarget::Delete(1),
    )
    .unwrap();
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))),
        "重建后仍等确认开始"
    );
    let events = answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap());
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

/// 讨论回合也要**逐片外送**：Chunk 的内容必须真的外送，否则界面整回合不动。
#[test]
pub(crate) fn discussion_turn_streams_deltas_and_never_leaks_the_envelope() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut chat = super::super::RecordingChat {
        // 正文在信封**之前**：正文可以逐片流出去，信封本身绝不能。
        inner: scripted(vec![
            "我说两句。{\"type\":\"say\",\"text\":\"我说两句\"}".into()
        ]),
        seen: Arc::clone(&seen),
    };
    let mut events: Vec<crate::capabilities::session::api::SessionEvent> = Vec::new();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prompts = test_prompts();
    let _ = crate::capabilities::collab::service::discussion::Discussion::turn_with(
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
    events.extend(answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap());
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    events.extend(answer_card(&mut core, &sid, OPT_PLAN_SAY, "同意开工").unwrap());
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

/// 核心操作必须走**工具调用**：正文里手写 JSON 不再被接受（正文 JSON 既无 schema 校验也不进工具台账）。
#[test]
pub(crate) fn core_operations_require_a_tool_call_not_body_json() {
    let systools = test_systools();
    // 角色表把核心操作发给对应的核心身份（越权校验与工具面的判据都是它）。
    for (role, tool) in [
        ("planner", "plan"),
        ("planner", "slate"),
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
/// 然后再要那一次核心操作调用（plan）。单次调用会让模型一想核实就被判"没有调用 plan"，
/// 整步中断。
#[test]
pub(crate) fn core_operation_runs_readonly_verification_before_the_op() {
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["demo", "work", "note.txt"]);
    io.seed(&["demo", "work", "note.txt"], "现场：一切正常\n");
    let sb = test_sandbox("核心", &[]);
    let mut verify = MemberTools {
        mode: crate::capabilities::llm::api::ToolMode::Envelope,
        role: "orchestrator".to_string(),
        modules: BTreeMap::new(),
        observations: crate::capabilities::tools::api::Observations::default(),
        llm: test_llm_demo(),
        log: Arc::new(crate::kernel::ports::NoopLog),
        tools: test_tools_svc_with(Arc::new(SilentRunner), io, Arc::new(NoFenceHost)),
        sandbox: sb.clone(),
        builtin_tools: test_systools().tools,
        unavailable: BTreeMap::new(),
        fence: crate::capabilities::tools::api::FenceSpec::from_sandbox(&sb, false),
        reply_seq: 0,
        line: Default::default(),
        allowed: vec!["read".to_string(), "plan".to_string()],
        with_modules: false,
        notes: crate::capabilities::tools::api::ToolNotes::default(),
        handlers: Vec::new(),
    };
    // 第一轮：先核实（read）；第二轮：交出 plan。
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut chat = super::super::RecordingChat {
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
    let out = crate::capabilities::session::api::core_operation(
        &*test_tools_svc(),
        "planner",
        "plan",
        crate::capabilities::llm::api::ToolMode::Envelope,
        &mut chat,
        &[crate::capabilities::llm::api::Msg::user("出方案")],
        crate::capabilities::llm::api::CompleteOpts::plain(false),
        None,
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
    let out = crate::capabilities::session::api::core_operation(
        &*test_tools_svc(),
        "planner",
        "plan",
        crate::capabilities::llm::api::ToolMode::Envelope,
        chat.as_mut(),
        &[crate::capabilities::llm::api::Msg::user("出方案")],
        crate::capabilities::llm::api::CompleteOpts::plain(false),
        None,
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
    let out = crate::capabilities::tools::service::systool::execute(
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
