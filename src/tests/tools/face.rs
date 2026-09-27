//! 工具面发放、原生通道与并发：本回合工具面、声明式并发与写入屏障
use super::super::builders::*;
use super::super::prelude::*;
/// **改形态不必重建会话**：会话里存的是**参数**（`SessionParams`），身份块每次调用现渲染。
/// 判据：同一个会话（没被重建）在建好之后，把登记处里的形态探测成 native——
/// 下一回合的请求已经换了一套调用约定，而对话（此前说过的话）一条没丢。
#[test]
pub(crate) fn tool_mode_change_needs_no_session_rebuild() {
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![vec![NativeStep::Text(
                "{\"type\":\"say\",\"text\":\"答一\"}".to_string(),
            )]]),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let before = core.single_identity(&sid).expect("身份块");
    assert!(
        before.contains("只输出一个 JSON 信封"),
        "建会话时是信封形态：{}",
        before
    );
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap();
    let said = core.single_history(&sid).unwrap();
    assert!(
        said.iter().any(|m| m.content.contains("问一")),
        "用户那句话在对话里：{:?}",
        said
    );

    // 登记处把模型判成原生（产品里就是「测工具调用」那一下）：形态不钉在会话里。
    core.registry_mut().probe_model_tools("m").expect("探测");
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap();
    let after = core.single_identity(&sid).expect("身份块");
    assert!(
        after.contains("原生工具调用"),
        "形态改了，身份块跟着改（不必重建会话）：{}",
        after
    );
    let dialogue = core.single_history(&sid).unwrap();
    assert!(
        dialogue.iter().any(|m| m.content.contains("问一")),
        "换形态不该丢对话：{:?}",
        dialogue
    );
    assert!(
        dialogue.iter().all(|m| !m.content.contains("【工作环境】")),
        "对话里不许出现身份块：{:?}",
        dialogue
    );
}

/// **总表不进提示词**：模型只看到"这一回合能用的工具"那一块；没拿到的不出现——
/// 提示词块与原生声明槽用的是**同一份 allowed**（两处一起收口，不然就是列了必然被拒的）。
#[test]
pub(crate) fn only_this_turns_tools_are_advertised() {
    let io = Arc::new(InMemorySysIo::new());
    let declared = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut m = native_member("a", io, vec![], Arc::clone(&declared), Arc::clone(&seen));
    let prompts = test_prompts();
    let _ = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    {
        let log = seen.lock().expect("锁");
        let systems: Vec<&str> = log[0]
            .iter()
            .filter(|x| x.role == "system")
            .map(|x| x.content.as_str())
            .collect();
        assert_eq!(systems.len(), 2, "身份 + 本回合工具块：{:?}", systems);
        assert!(
            !systems[0].contains("【本回合可用的工具】") && !systems[0].contains("offset（integer"),
            "系统身份里不许出现工具清单：{}",
            systems[0]
        );
        assert!(
            systems[1].contains("【本回合可用的工具】"),
            "工具块随回合注入：{}",
            systems[1]
        );
    }
    // 换成"只拿到 read"的席位：提示词块与声明槽同时收口。
    m.tools.as_mut().expect("工具环境").allowed = vec!["read".to_string()];
    let _ = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    let log = seen.lock().expect("锁");
    let block = log[1]
        .iter()
        .find(|x| x.role == "system" && x.content.contains("【本回合可用的工具】"))
        .expect("工具块");
    assert!(
        block.content.contains("- path（string，必填）"),
        "{}",
        block.content
    );
    assert!(
        !block.content.contains("patch") && !block.content.contains("submit_report"),
        "没拿到的工具不进工具块：{}",
        block.content
    );
    let decls = declared.lock().expect("锁");
    let names: Vec<&str> = decls[1].iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["read"],
        "声明槽与 allowed 同一份判据：{:?}",
        names
    );
}

#[test]
pub(crate) fn native_mode_declares_tools_and_runs_multiple_structured_calls() {
    use crate::capabilities::llm::api::ToolCall;
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["demo", "work", "note.txt"], "第一行\n第二行\n");
    let note = s(&["demo", "work", "note.txt"]);
    let declared = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut m = native_member(
        "a",
        Arc::clone(&io),
        vec![
            // 一次回复里两个互不依赖的调用（原生协议本来就是数组）
            NativeStep::Calls(vec![
                ToolCall {
                    id: "c1".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", note),
                },
                ToolCall {
                    id: "c2".to_string(),
                    name: "search".to_string(),
                    args_json: format!("{{\"path\":\"{}\",\"keyword\":\"第二\"}}", note),
                },
            ]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
        ],
        Arc::clone(&declared),
        Arc::clone(&seen),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(
        exec.reports.get("a").map(|s| s.as_str()),
        Some("读完了"),
        "两个调用跑完后回到文本轮"
    );

    // ① 声明真的发出去了：内置五个工具一个不少；patch 的参数契约是 body（不是信封里的 args）
    let decls = declared.lock().expect("锁");
    let first = &decls[0];
    let names: Vec<&str> = first.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["read", "write", "edit", "patch", "search"] {
        assert!(names.contains(&want), "没声明 {}：{:?}", want, names);
    }
    let patch = first
        .iter()
        .find(|(n, _)| n == "patch")
        .expect("patch 的声明")
        .1
        .clone();
    assert_eq!(
        patch["required"],
        serde_json::json!(["body"]),
        "patch 在原生通道上用 body 承载补丁正文"
    );
    assert_eq!(patch["additionalProperties"], serde_json::json!(false));
    // read 的声明来自 prompts/ 的声明（含 path 必填）
    let read = first
        .iter()
        .find(|(n, _)| n == "read")
        .expect("read 的声明")
        .1
        .clone();
    assert_eq!(read["required"], serde_json::json!(["path"]));

    // ② 两个调用各成一条工具行，结果按原顺序回填
    let trace = exec.traces.get("a").expect("工具调用应入册");
    assert_eq!(
        trace.len(),
        2,
        "一次两个调用 = 两条工具行：{:?}",
        trace.iter().map(|v| &v.name).collect::<Vec<_>>()
    );
    assert!(
        trace[0].ok && trace[0].name == "read" && trace[0].output.contains("第一行"),
        "{:?}",
        trace[0]
    );
    assert!(
        trace[1].ok && trace[1].name == "search" && trace[1].output.contains("2 | 第二行"),
        "{:?}",
        trace[1]
    );

    // ③ 第二轮请求里：**一条**助手消息带两个 tool_calls，后面跟两条 role=tool（协议形状）
    let msgs = seen.lock().expect("锁");
    let second = &msgs[1];
    let with_calls: Vec<&Msg> = second.iter().filter(|m| !m.tool_calls.is_empty()).collect();
    assert_eq!(
        with_calls.len(),
        1,
        "一次回复只推一条助手消息（多个调用都挂在它上面）：{:?}",
        second
            .iter()
            .map(|m| (m.role.clone(), m.tool_calls.len()))
            .collect::<Vec<_>>()
    );
    let calls = &with_calls[0].tool_calls;
    assert_eq!(calls.len(), 2, "两个调用都挂在这条助手消息上");
    assert_eq!(calls[0].id, "c1");
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].args_json, format!("{{\"path\":\"{}\"}}", note));
    assert_eq!(calls[1].id, "c2");
    assert_eq!(calls[1].name, "search");
    let results: Vec<&Msg> = second.iter().filter(|m| m.role == "tool").collect();
    assert_eq!(
        results.len(),
        2,
        "每条调用一条 role=tool 的结果：{:?}",
        second
    );
    assert_eq!(
        results[0].tool_call_id, "c1",
        "结果靠 tool_call_id 回应它的调用"
    );
    assert_eq!(results[1].tool_call_id, "c2");
    assert!(
        results[0].content.contains("[工具结果] read") && results[0].content.contains("第一行"),
        "{:?}",
        results[0]
    );
    assert!(
        results[1].content.contains("2 | 第二行"),
        "{:?}",
        results[1]
    );
}

#[test]
pub(crate) fn native_mode_refuses_a_hand_written_envelope() {
    let io = Arc::new(InMemorySysIo::new());
    let target = s(&["demo", "work", "out.md"]);
    let envelope = format!(
        "正文先写着。{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"x\"}}}}",
        target
    );
    let mut m = native_member(
        "a",
        Arc::clone(&io),
        vec![
            NativeStep::Text(envelope),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"知道了\"}".to_string()),
        ],
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    let trace = exec.traces.get("a").expect("应记一条失败的工具行");
    assert_eq!(trace.len(), 1);
    assert!(!trace[0].ok, "原生模式下信封不执行");
    assert_eq!(
        trace[0].output,
        prompts.tools().native_no_envelope,
        "要如实说清本通道用原生调用"
    );
    assert_eq!(io.get(&["demo", "work", "out.md"]), None, "绝不落盘");
}

/// 声明可并发的读取**真的并发**，且结果一律按**原始调用顺序**回填。
/// 第一个文件故意慢：它会**后完成**，但结果仍必须排在前面（上下文里不许乱序）。
#[test]
pub(crate) fn declared_parallel_reads_overlap_and_results_keep_the_call_order() {
    use crate::capabilities::llm::api::ToolCall;
    let io = Arc::new(InMemorySysIo::new().slow(40));
    io.seed(&["demo", "work", "a.txt"], "A1\nA2\n");
    io.seed(&["demo", "work", "b.txt"], "B1\nB2\n");
    // 先发的读 a.txt 慢 120ms（后完成），后发的读 b.txt 快——顺序只能靠"按原始下标回填"保住。
    io.slow_file(&["demo", "work", "a.txt"], 120);
    let a = s(&["demo", "work", "a.txt"]);
    let b = s(&["demo", "work", "b.txt"]);
    let call = |id: &str, path: &str| ToolCall {
        id: id.to_string(),
        name: "read".to_string(),
        args_json: format!("{{\"path\":\"{}\"}}", path),
    };
    let mut m = native_member(
        "a",
        Arc::clone(&io),
        vec![
            NativeStep::Calls(vec![call("c1", &a), call("c2", &b)]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
        ],
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    let peak = io.peak_concurrent_reads();
    assert!(
        peak >= 2,
        "册子声明 parallel 的读取要真的并发（峰值 {}）",
        peak
    );
    let trace = exec.traces.get("a").expect("两条工具行");
    assert_eq!(trace.len(), 2);
    assert!(
        trace[0].output.contains("A1") && trace[1].output.contains("B1"),
        "结果按原始调用顺序回填（慢的那个也排在前面）：{:?}",
        trace
            .iter()
            .map(|v| v.output.chars().take(20).collect::<String>())
            .collect::<Vec<_>>()
    );
}

/// 写入类独占执行：它是并发批次之间的**屏障**，并且能看到并发批次**合并后**的账本。
#[test]
pub(crate) fn a_writing_call_is_a_barrier_and_sees_the_merged_ledger() {
    use crate::capabilities::llm::api::ToolCall;
    let io = Arc::new(InMemorySysIo::new().slow(30));
    io.seed(&["demo", "work", "a.txt"], "原文\n");
    let a = s(&["demo", "work", "a.txt"]);
    let mut m = native_member(
        "a",
        Arc::clone(&io),
        vec![
            NativeStep::Calls(vec![
                ToolCall {
                    id: "c1".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", a),
                },
                // 整份覆盖要求"本次会话完整读过"：这条证据只能来自上面并发批次的合并。
                ToolCall {
                    id: "c2".to_string(),
                    name: "write".to_string(),
                    args_json: format!("{{\"path\":\"{}\",\"content\":\"新内容\\n\"}}", a),
                },
            ]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"改好了\"}".to_string()),
        ],
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    let trace = exec.traces.get("a").expect("两条工具行");
    assert_eq!(trace.len(), 2);
    assert!(
        trace[1].ok,
        "写入类排在并发批次之后，且能看到合并后的账本：{}",
        trace[1].output
    );
    assert_eq!(
        io.get(&["demo", "work", "a.txt"]).as_deref(),
        Some("新内容\n")
    );
    assert_eq!(
        io.peak_concurrent_reads(),
        1,
        "写入类是屏障：它绝不与只读批次重叠"
    );
}

/// 模块工具的可并发性是**模块作者在 module.yaml 里的声明**：声明了才并发，没声明一律串行。
#[test]
pub(crate) fn module_tools_are_concurrent_only_when_declared() {
    use crate::capabilities::llm::api::ToolCall;
    /// 原生形态 + 模块 m0 声明外部工具 grep（线上名 m0_grep）；parallel 决定它是否可并发。
    fn grep_member(runner: Arc<dyn ToolRunner + Send + Sync>, parallel: bool) -> Member {
        let steps = vec![
            NativeStep::Calls(vec![
                ToolCall {
                    id: "c1".to_string(),
                    name: "m0_grep".to_string(),
                    args_json: "{\"keyword\":\"甲\"}".to_string(),
                },
                ToolCall {
                    id: "c2".to_string(),
                    name: "m0_grep".to_string(),
                    args_json: "{\"keyword\":\"乙\"}".to_string(),
                },
            ]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"查完了\"}".to_string()),
        ];
        let mut m = native_member(
            "a",
            Arc::new(InMemorySysIo::new()),
            steps,
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(Mutex::new(Vec::new())),
        );
        let t = m.tools.as_mut().expect("工具环境");
        let mt = t.modules.get_mut("m0").expect("模块");
        mt.commands
            .insert("grep".to_string(), "python tools/grep.py".to_string());
        if parallel {
            mt.parallel.insert("grep".to_string());
        }
        t.tools = test_tools_svc_with(
            runner,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        );
        m
    }
    let prompts = test_prompts();
    // ① 声明 parallel：两个调用真的并发
    let runner = Arc::new(ParallelRunner::new(40));
    let mut m = grep_member(
        Arc::clone(&runner) as Arc<dyn ToolRunner + Send + Sync>,
        true,
    );
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    let peak = runner.peak_concurrent();
    assert!(
        peak >= 2,
        "声明 parallel 的模块工具要真的并发（峰值 {}）",
        peak
    );
    assert_eq!(exec.traces.get("a").map(|t| t.len()), Some(2));
    // ② 没声明：同样两个调用逐个跑
    let runner = Arc::new(ParallelRunner::new(5));
    let mut m = grep_member(
        Arc::clone(&runner) as Arc<dyn ToolRunner + Send + Sync>,
        false,
    );
    let _ = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(
        runner.peak_concurrent(),
        1,
        "未声明可并发 = 独占串行（峰值 {}）",
        runner.peak_concurrent()
    );
}

#[test]
pub(crate) fn changing_the_declared_mode_takes_effect_on_the_next_generation() {
    use crate::capabilities::conductor::api::ProbeOutcome;
    // 一开始登记处说"不支持原生"：会话按手写信封装配（系统提示也就教信封）
    let outcome = Arc::new(Mutex::new(ProbeOutcome::Unsupported {
        detail: "先不支持".to_string(),
    }));
    let mut core = core_with_gateway(
        vec![module_of("a")],
        ProbeGateway {
            outcome: Arc::clone(&outcome),
        },
    );
    core.registry_mut()
        .provider_upsert("p", "http://x", "k")
        .expect("登记供应商");
    core.registry_mut()
        .model_upsert("m", "M", "api-m", "p", "", 0)
        .expect("登记模型");
    let sid = core
        .create_work(WorkSpec {
            name: "w".to_string(),
            mode: WorkMode::Single,
            agents: vec![AgentInstance {
                name: "a".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: Some("m".to_string()),
            }],
            task: None,
            delegate: false,
        })
        .expect("建会话")
        .sid;
    let notes = |ev: &Vec<SessionEvent>| -> Vec<String> {
        ev.iter()
            .filter_map(|e| match e {
                SessionEvent::Notice(n) => Some(n.clone()),
                _ => None,
            })
            .collect()
    };
    // ① 没动登记处：不该有任何形态通知
    let e1 = with_live(|l| core.single_say(&sid, "你好", l)).expect("发言");
    assert!(
        !notes(&e1).iter().any(|n| n.contains("形态已按登记处")),
        "没改登记处就不该重新查：{:?}",
        notes(&e1)
    );
    // ② 用户改了登记处（探测确认支持）→ 下一次生成前重新解析、就地刷新、如实通知
    *outcome.lock().expect("锁") = ProbeOutcome::Supported {
        detail: "支持".to_string(),
    };
    core.registry_mut().probe_model_tools("m").expect("探测");
    let e2 = with_live(|l| core.single_say(&sid, "再问", l)).expect("发言");
    assert!(
        notes(&e2).iter().any(|n| n.contains("原生工具调用")),
        "形态变了要如实通知：{:?}",
        notes(&e2)
    );
    // ③ 形态没再变：不重复通知（没动就不管）
    let e3 = with_live(|l| core.single_say(&sid, "又问", l)).expect("发言");
    assert!(
        !notes(&e3).iter().any(|n| n.contains("形态已按登记处")),
        "形态没变不该重复通知：{:?}",
        notes(&e3)
    );
}
