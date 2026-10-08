//! 工具面与调度：同名冲突、越权拒绝、事件顺序
use super::super::builders::*;
use super::super::prelude::*;
#[test]
pub(crate) fn same_named_tools_across_modules_are_no_longer_a_conflict() {
    let mut a = module_of("a");
    a.manifest
        .tools
        .insert("dump".to_string(), decl("python a/dump.py"));
    let mut b = module_of("b");
    b.manifest
        .tools
        .insert("dump".to_string(), decl("python b/dump.py"));
    let mut core = core_with(vec![a, b], gw(BTreeMap::new(), vec!["[]".into()]));
    // 跨模块同名工具不再冲突：信封里的 module 消歧（行为见 same_named_tools_in_two_modules_run_in_their_own_root）。
    let one_agent = WorkSpec {
        name: "w".to_string(),
        mode: WorkMode::Single,
        agents: vec![AgentInstance {
            name: "组合".to_string(),
            transient: true,
            modules: vec!["a".to_string(), "b".to_string()],
            model: None,
        }],
        task: None,
        delegate: false,
        tier: crate::kernel::api::Tier::Host,
    };
    assert!(
        core.create_work(one_agent).is_ok(),
        "同名工具同属一个 agent 也应能建"
    );
    // 仍然保留的校验：同一模块不得同时属于两个 agent（沙箱与发言归属会歧义）。
    let cross = WorkSpec {
        name: "cross".to_string(),
        mode: WorkMode::Collab,
        agents: vec![
            AgentInstance {
                name: "甲".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
            },
            AgentInstance {
                name: "乙".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
            },
        ],
        task: Some("需求".to_string()),
        delegate: false,
        tier: crate::kernel::api::Tier::Host,
    };
    assert!(core
        .create_work(cross)
        .unwrap_err()
        .contains("被多个 agent"));
}

/// 两个模块可以声明**同名**工具（信封里的 module 消歧）；每次调用跑在各自模块的目录里。
#[test]
pub(crate) fn same_named_tools_in_two_modules_run_in_their_own_root() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut member = BTreeMap::new();
    member.insert(
        "组合".to_string(),
        vec![
            "{\"type\":\"tool\",\"module\":\"a\",\"name\":\"read_txt\",\"args\":{}}".to_string(),
            "{\"type\":\"tool\",\"module\":\"b\",\"name\":\"read_txt\",\"args\":{}}".to_string(),
            "{\"type\":\"say\",\"text\":\"两份都读完了\"}".to_string(),
        ],
    );
    let mut a = module_of("a");
    a.root = abs(&["mods", "a"]);
    a.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let mut b = module_of("b");
    b.root = abs(&["mods", "b"]);
    b.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let mut core = core_with_runner(
        vec![a, b],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    // 跨模块同名不再算冲突：照样能建工作。
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a", "b"]))
        .unwrap()
        .sid;
    // 模块工具清单**不在系统提示里**（随回合注入）：分组形态由
    // module_tool_params_are_declared_in_the_manifest_and_enforced_by_core 盯。
    let identity = core.single_identity(&sid).expect("身份块");
    assert!(
        !identity.contains("read_txt"),
        "系统提示里不许出现工具清单：{}",
        identity
    );
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    // 信封写了 module → tool 行呈现成 模块.工具（用户一眼看出调的是谁的）。
    let tool_lines = tool_line_texts(&events);
    assert_eq!(
        tool_lines.len(),
        2,
        "两次调用 = 两条 tool 行：{:?}",
        tool_lines
    );
    assert!(tool_lines[0].contains("a.read_txt"), "{:?}", tool_lines);
    assert!(tool_lines[1].contains("b.read_txt"), "{:?}", tool_lines);
    assert!(
        tool_lines.iter().all(|l| l.contains("成功")),
        "{:?}",
        tool_lines
    );
    let calls = runner.calls.lock().expect("锁").clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, "python tools/read_txt.py");
    assert_eq!(
        calls[0].0,
        abs(&["mods", "a"]),
        "module=a 的 read_txt 跑在 a 的目录"
    );
    assert_eq!(calls[1].1, "python tools/read_txt.py");
    assert_eq!(
        calls[1].0,
        abs(&["mods", "b"]),
        "module=b 的同名工具跑在 b 的目录（不共用 a 的 cwd）"
    );
}

/// 多模块 agent 下省略 module：核心不猜，如实报错并把可用的「模块.工具」列全。
#[test]
pub(crate) fn external_tool_without_module_is_refused_when_agent_has_many_modules() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut member = BTreeMap::new();
    member.insert(
        "组合".to_string(),
        vec![
            "{\"type\":\"tool\",\"name\":\"read_txt\",\"args\":{}}".to_string(),
            "{\"type\":\"say\",\"text\":\"知道了\"}".to_string(),
        ],
    );
    let mut a = module_of("a");
    a.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let mut b = module_of("b");
    b.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let mut core = core_with_runner(
        vec![a, b],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a", "b"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "不猜模块，绝不落进程"
    );
    let h = core.single_history(&sid).unwrap();
    let feedback = h
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[工具结果] read_txt"))
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(feedback.contains("没有指明所属模块"), "{}", feedback);
    // 失败也要落一条 tool 行（用户看得到这次调用没成）。
    let lines = tool_line_texts(&events);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("read_txt") && l.contains("失败")),
        "失败的调用同样要发 tool 行：{:?}",
        lines
    );
    assert!(
        feedback.contains("a.read_txt") && feedback.contains("b.read_txt"),
        "要把可用的 模块.工具 列全：{}",
        feedback
    );
    assert!(
        feedback.contains("read") && feedback.contains("write"),
        "内置工具也要列出：{}",
        feedback
    );
}

#[test]
pub(crate) fn tool_call_event_is_emitted_before_the_next_round() {
    // 短暂事件：工具跑完立刻发 tool_call（不落盘），供活动会话实时刷新。
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"完成\"}".into(),
        ],
    );
    let mut mod_a = module_of("a");
    mod_a
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), runner);
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let mut seen: Vec<&str> = Vec::new();
    {
        let mut emit = |e: SessionEvent| {
            seen.push(match e {
                SessionEvent::ToolCall(_) => "tool_call",
                SessionEvent::Transcript(_) => "transcript",
                SessionEvent::Notice(_) => "notice",
                SessionEvent::Delta { .. } => "delta",
                _ => "other",
            });
        };
        let mut live = Live {
            llm: Default::default(),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            emit: &mut emit,
            decisions: None,
            ask: None,
        };
        core.single_say(&sid, "跑一下", &mut live).unwrap();
    }
    assert!(
        seen.contains(&"tool_call"),
        "工具调用要发短暂 tool_call 事件：{:?}",
        seen
    );
}

#[test]
pub(crate) fn tool_type_mention_in_plain_speech_is_not_misjudged() {
    // ① 合法 say 信封的正文里引用 {"type":"tool"}：正常发言，不是工具调用。
    let say = serde_json::json!({"type": "say", "text": "调用格式是 {\"type\":\"tool\"} 这样"})
        .to_string();
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![say]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "怎么写", l)).unwrap();
    assert!(
        tool_views(&events).is_empty(),
        "不该有工具行：{:?}",
        transcript_rows(&events)
    );
    assert_eq!(
        transcript_rows(&events).iter().filter(|r| !r.2).count(),
        2,
        "用户行 + 一条发言"
    );

    // ② 普通正文（不以 { 开头）里提到它：仍按发言收录（降级），不误判。
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["写法示例：{\"type\":\"tool\"} 就是这样".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "再讲一次", l)).unwrap();
    assert!(
        tool_views(&events).is_empty(),
        "不以 JSON 为主体 → 不误判：{:?}",
        transcript_rows(&events)
    );
    assert!(
        transcript_rows(&events)
            .iter()
            .any(|r| r.1.contains("写法示例")),
        "{:?}",
        transcript_rows(&events)
    );
    // ③ 正文里引用一个**平衡**的完整对象：同样不误判（未闭合才判坏信封）。
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["格式是 {\"type\":\"tool\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "再示范一次", l)).unwrap();
    assert!(
        tool_views(&events).is_empty(),
        "平衡对象不算坏信封：{:?}",
        transcript_rows(&events)
    );
    assert!(
        transcript_rows(&events)
            .iter()
            .any(|r| r.1.contains("格式是")),
        "{:?}",
        transcript_rows(&events)
    );
}

#[test]
pub(crate) fn tool_loop_runs_declared_tool() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "  3 | 命中行".into(),
        ok: true,
    });
    let mut m = member_with_tools(
        "m0",
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"完成\"}".into(),
        ],
        Arc::clone(&runner),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("完成"));
    let calls = runner.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1, "声明过的工具应恰好执行一次");
    assert_eq!(
        calls[0].0,
        abs(&["mods", "root"]),
        "工具进程工作目录 = 模块工作区（真实路径）"
    );
    assert_eq!(calls[0].1, "python tools/grep.py", "命令来自模块清单");
    assert!(calls[0].2.contains("keyword"), "参数以 JSON 原样送达");
    let trace = exec.traces.get("m0").expect("工具调用应入册");
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0].name, "grep");
    assert_eq!(trace[0].module, "m0", "信封省略 module 时按唯一模块兜底");
    assert!(
        trace[0].ok && trace[0].args.contains("keyword"),
        "{:?}",
        trace[0]
    );
    assert!(
        trace[0].raw.contains("\"type\":\"tool\""),
        "原始输出要留档（重建上下文用）"
    );
}
