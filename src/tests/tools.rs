//! 工具能力测试：内置工具与参数契约、工具面发放、并发与越权、补丁通道。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

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

#[test]
pub(crate) fn module_tool_params_are_declared_in_the_manifest_and_enforced_by_core() {
    // 参数契约写在 module.yaml（不埋进代码）：核心按它校验，并把说明写进系统提示。
    let mut mod_m0 = module_of("m0");
    mod_m0.manifest.tools.insert(
        "grep".to_string(),
        decl_with(
            "python tools/grep.py",
            "keyword: {type: string, required: true}\n",
        ),
    );
    let prompts = test_prompts();
    let system = crate::capabilities::workspace::api::agent_system(
        &prompts,
        "m0",
        &[(mod_m0.manifest.id.clone(), mod_m0.manifest.system.clone())],
        "工具说明",
        crate::capabilities::llm::api::ToolMode::Envelope,
    );
    // 模块工具清单与参数**不进系统提示**：随回合注入（能不能用模块工具由角色表的 module_tools 决定）。
    assert!(
        !system.contains("【模块工具参数】"),
        "系统提示里不许出现模块工具清单：{}",
        system
    );
    let notes = crate::tests::doubles::test_notes(
        &test_sandbox("m0", &["m0"]),
        std::slice::from_ref(&mod_m0),
    );
    assert!(
        notes.module_tool_params.contains("【模块工具参数】")
            && notes.module_tools.contains("- m0：grep"),
        "模块工具说明由装配期算好、随回合注入：{:?}",
        notes
    );
    assert!(
        notes.module_tool_params.contains("keyword（string，必填）"),
        "{:?}",
        notes
    );
    // 跨模块**同名**工具：清单按模块分组（模型照此写信封里的 module）。
    let mut mod_a = module_of("a");
    mod_a
        .manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let mut mod_b = module_of("b");
    mod_b
        .manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    let pair =
        crate::tests::doubles::test_notes(&test_sandbox("组合", &["a", "b"]), &[mod_a, mod_b]);
    assert!(
        pair.module_tools.contains("- a：read_txt") && pair.module_tools.contains("- b：read_txt"),
        "清单要按模块分组：{}",
        pair.module_tools
    );

    let table = crate::capabilities::session::api::tool_table(std::slice::from_ref(&mod_m0));
    let books = table.get("m0").expect("放行表").books.clone();
    assert_eq!(books.len(), 1, "只给声明了参数的工具建契约");

    // 参数不符：拒收，且说清缺哪个参数、并把工具签名发回（不启动进程）。
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut m = member_with_tools(
        "m0",
        vec![
            "{\"type\":\"tool\",\"name\":\"grep\",\"args\":{}}".to_string(),
            "{\"type\":\"say\",\"text\":\"完成\"}".to_string(),
        ],
        Arc::clone(&runner),
    );
    m.tools
        .as_mut()
        .expect("工具环境")
        .modules
        .get_mut("m0")
        .expect("模块")
        .books = books;
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "参数不合法绝不落进程"
    );
    let trace = exec.traces.get("m0").expect("失败的调用也要入册");
    assert!(
        trace[0].output.contains("缺少必填参数 keyword"),
        "{}",
        trace[0].output
    );
    assert!(
        trace[0].output.contains("keyword（string，必填）"),
        "失败要带上参数签名：{}",
        trace[0].output
    );

    // 参数合法：照旧执行，args 原样交给工具。
    let runner2 = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut m2 = member_with_tools(
        "m0",
        vec![
            TOOL_CALL.to_string(),
            "{\"type\":\"say\",\"text\":\"完成\"}".to_string(),
        ],
        Arc::clone(&runner2),
    );
    let books2 = table.get("m0").expect("放行表").books.clone();
    m2.tools
        .as_mut()
        .expect("工具环境")
        .modules
        .get_mut("m0")
        .expect("模块")
        .books = books2;
    run_execution(std::slice::from_mut(&mut m2), "任务", &prompts);
    let calls = runner2.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1, "合法调用照常执行");
    assert!(calls[0].2.contains("keyword"));

    // 没声明参数的工具照旧不校验（不给模块开发者添门槛）。
    let plain = module_of("m0");
    let plain_table = crate::capabilities::session::api::tool_table(std::slice::from_ref(&plain));
    assert!(
        plain_table.get("m0").expect("放行表").books.is_empty(),
        "没声明参数 = 没有契约"
    );
}

#[test]
pub(crate) fn tool_loop_rejects_undeclared_tool() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: String::new(),
        ok: true,
    });
    let mut m = member_with_tools(
        "m0",
        vec![
            "{\"type\":\"tool\",\"name\":\"nope\",\"args\":{}}".into(),
            "{\"type\":\"say\",\"text\":\"完成\"}".into(),
        ],
        Arc::clone(&runner),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "未声明的工具绝不落进程"
    );
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("完成"));
    let trace = exec.traces.get("m0").expect("工具调用应入册");
    assert!(
        trace.iter().any(|v| v.output.contains("未声明")),
        "{:?}",
        trace.iter().map(|v| &v.output).collect::<Vec<_>>()
    );
}

#[test]
pub(crate) fn tool_loop_has_no_call_cap() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "r".into(),
        ok: true,
    });
    const N: usize = 10; // 远大于任何合理上限：证明没有调用次数上限
    let mut script: Vec<String> = (0..N).map(|_| TOOL_CALL.to_string()).collect();
    script.push("{\"type\":\"say\",\"text\":\"最终回报\"}".into());
    let mut m = member_with_tools("m0", script, Arc::clone(&runner));
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        N,
        "几次调用就几次（没有封顶）"
    );
    assert_eq!(
        exec.reports.get("m0").map(|s| s.as_str()),
        Some("最终回报"),
        "模型给出 say 才收尾"
    );
}

#[test]
pub(crate) fn builtin_search_reports_line_numbers_and_respects_case() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(
        &["demo", "work", "note.txt"],
        "第一行 Alpha\n第二行 beta\nalpha 小写\n",
    );
    let path = s(&["demo", "work", "note.txt"]);
    // 默认区分大小写
    let out = run_builtin(
        &sb,
        &io,
        "search",
        &format!("{{\"path\":\"{}\",\"keyword\":\"alpha\"}}", path),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("3 | alpha 小写"), "{}", out.output);
    assert!(
        !out.output.contains("第一行 Alpha"),
        "默认区分大小写：{}",
        out.output
    );
    assert!(
        out.output.contains("命中 1 行 / 全文 3 行"),
        "{}",
        out.output
    );
    // ignore_case = true
    let out = run_builtin(
        &sb,
        &io,
        "search",
        &format!(
            "{{\"path\":\"{}\",\"keyword\":\"alpha\",\"ignore_case\":true}}",
            path
        ),
    );
    assert!(
        out.output.contains("1 | 第一行 Alpha") && out.output.contains("3 | alpha 小写"),
        "{}",
        out.output
    );
    assert!(
        out.output.contains("命中 2 行 / 全文 3 行"),
        "{}",
        out.output
    );
    // 越界被拒
    let bad = run_builtin(
        &sb,
        &io,
        "search",
        &format!(
            "{{\"path\":\"{}\",\"keyword\":\"x\"}}",
            s(&["outside", "f.txt"])
        ),
    );
    assert!(
        !bad.ok && bad.output.contains("不在允许的根目录内"),
        "{}",
        bad.output
    );
}

#[test]
pub(crate) fn builtin_file_tools_run_in_direct_session() {
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["w", "a", "note.txt"]);
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            format!("{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"你好\"}}}}", note),
            format!("{{\"type\":\"tool\",\"name\":\"read\",\"args\":{{\"path\":\"{}\"}}}}", note),
            "{\"type\":\"say\",\"text\":\"已读写完\"}".to_string(),
        ],
    );
    let mut core = core_with_io(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    // 落点 = session/<工作名>/<agent 实例名>/（测试里的临时 agent 名取模块名）
    assert_eq!(io.get(&["w", "a", "note.txt"]).as_deref(), Some("你好"));
    let tool_lines = tool_line_texts(&events);
    assert_eq!(
        tool_lines.len(),
        2,
        "write + read = 两条 tool 行：{:?}",
        tool_lines
    );
    assert!(
        tool_lines[0].contains("write") && tool_lines[1].contains("read"),
        "{:?}",
        tool_lines
    );
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter().any(|m| m.role == "user"
            && m.content.contains("[工具结果] read")
            && m.content.contains("你好")),
        "读回的内容必须回注上下文"
    );
}

#[test]
pub(crate) fn builtin_write_into_module_dir_is_allowed_with_notice() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &["data"]);
    let keep = s(&["mods", "data", "keep.txt"]);
    let out = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"状态\"}}", keep),
    );
    assert!(out.ok, "{}", out.output);
    assert_eq!(
        io.get(&["mods", "data", "keep.txt"]).as_deref(),
        Some("状态")
    );
    assert!(
        out.output.contains("模块 data"),
        "写模块目录要如实提示：{}",
        out.output
    );
    assert!(
        out.output
            .contains(crate::capabilities::tools::domain::systool::MODULE_WRITE_MARK),
        "要有可供轨迹识别的标记：{}",
        out.output
    );
    // 越界写入：拒绝且不落盘。
    let other = s(&["mods", "other", "x.txt"]);
    let bad = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", other),
    );
    assert!(!bad.ok);
    assert_eq!(io.get(&["mods", "other", "x.txt"]), None);
}

#[test]
pub(crate) fn builtin_edit_replaces_the_requested_span_and_reports_what_it_did() {
    let io = Arc::new(InMemorySysIo::new());
    let sb = test_sandbox("a1", &[]);
    let note = s(&["demo", "work", "note.txt"]);
    io.seed(
        &["demo", "work", "note.txt"],
        "第一段\n要改的句子\n第三段\n",
    );
    let mut obs = crate::capabilities::tools::api::Observations::default();
    let exec = test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::clone(&io),
        Arc::new(NoFenceHost),
    );
    let edit = |obs: &mut crate::capabilities::tools::api::Observations, args: &str| {
        let full = format!("{{\"path\":\"{}\",{}}}", note, args);
        exec.run_builtin(&sb, &test_systools().tools, obs, "edit", &full)
    };
    // 唯一命中：只改那一处，别处一字不动
    let ok = edit(
        &mut obs,
        "\"old_string\":\"要改的句子\",\"new_string\":\"改好了\"",
    );
    assert!(ok.ok, "{}", ok.output);
    assert_eq!(
        io.get(&["demo", "work", "note.txt"]).as_deref(),
        Some("第一段\n改好了\n第三段\n")
    );
    assert!(ok.output.contains("替换 1 处"), "{}", ok.output);
    // old == new：什么都不会变，如实拒绝
    let same = edit(
        &mut obs,
        "\"old_string\":\"改好了\",\"new_string\":\"改好了\"",
    );
    assert!(
        !same.ok && same.output.contains("什么都不会变"),
        "{}",
        same.output
    );
    // 找不到：说清没找到，并指出"只差空白"的那一行（模型据此改对缩进）
    let miss = edit(
        &mut obs,
        "\"old_string\":\"   改好了\",\"new_string\":\"x\"",
    );
    assert!(
        !miss.ok && miss.output.contains("没找到 old_string"),
        "{}",
        miss.output
    );
    assert!(
        miss.output.contains("第 2 行与它只差空白"),
        "{}",
        miss.output
    );
    // 多处命中：列出位置，且绝不写盘
    io.seed(&["demo", "work", "note.txt"], "dup\ndup\n");
    let multi = edit(&mut obs, "\"old_string\":\"dup\",\"new_string\":\"x\"");
    assert!(
        !multi.ok && multi.output.contains("命中 2 处") && multi.output.contains("第 1、2 行"),
        "{}",
        multi.output
    );
    assert_eq!(
        io.get(&["demo", "work", "note.txt"]).as_deref(),
        Some("dup\ndup\n"),
        "拒收时绝不写盘"
    );
    // replace_all：全改
    let all = edit(
        &mut obs,
        "\"old_string\":\"dup\",\"new_string\":\"x\",\"replace_all\":true",
    );
    assert!(all.ok && all.output.contains("替换 2 处"), "{}", all.output);
    assert_eq!(
        io.get(&["demo", "work", "note.txt"]).as_deref(),
        Some("x\nx\n")
    );
    // new_string 空串 = 把这一段删掉
    let del = edit(
        &mut obs,
        "\"old_string\":\"x\",\"new_string\":\"\",\"replace_all\":true",
    );
    assert!(del.ok, "{}", del.output);
    assert_eq!(
        io.get(&["demo", "work", "note.txt"]).as_deref(),
        Some("\n\n")
    );
}

#[test]
pub(crate) fn builtin_edit_refuses_files_it_cannot_see_whole() {
    // 只看到开头 / 看到的是替换字符：改写会把没读到的内容或原始字节一起弄丢 → 一律不写盘。
    let note = s(&["demo", "work", "note.txt"]);
    let args = format!(
        "{{\"path\":\"{}\",\"old_string\":\"a\",\"new_string\":\"b\"}}",
        note
    );
    let sb = test_sandbox("a1", &[]);
    for (io, want) in [
        (
            Arc::new(InMemorySysIo::new().marked(false, true)),
            "超过单次读取上限",
        ),
        (
            Arc::new(InMemorySysIo::new().marked(true, false)),
            "非法 UTF-8",
        ),
    ] {
        io.seed(&["demo", "work", "note.txt"], "abc");
        let mut obs = crate::capabilities::tools::api::Observations::default();
        let exec = test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::clone(&io),
            Arc::new(NoFenceHost),
        );
        let out = exec.run_builtin(&sb, &test_systools().tools, &mut obs, "edit", &args);
        assert!(!out.ok && out.output.contains(want), "{}", out.output);
        assert_eq!(
            io.get(&["demo", "work", "note.txt"]).as_deref(),
            Some("abc"),
            "拒绝时不写盘"
        );
    }
}

#[test]
pub(crate) fn builtin_write_needs_a_complete_prior_read_of_an_existing_file() {
    let io = Arc::new(InMemorySysIo::new());
    let sb = test_sandbox("a1", &[]);
    let note = s(&["demo", "work", "note.txt"]);
    let exec = test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::clone(&io),
        Arc::new(NoFenceHost),
    );
    let run =
        |obs: &mut crate::capabilities::tools::api::Observations, tool: &str, args: String| {
            exec.run_builtin(&sb, &test_systools().tools, obs, tool, &args)
        };
    let mut obs = crate::capabilities::tools::api::Observations::default();
    // 新建文件：不需要"读过"什么
    let made = run(
        &mut obs,
        "write",
        format!("{{\"path\":\"{}\",\"content\":\"第一版\"}}", note),
    );
    assert!(made.ok, "{}", made.output);
    // 核心自己写过的文件：账本里有它的内容指纹 → 可以直接再写
    let again = run(
        &mut obs,
        "write",
        format!(
            "{{\"path\":\"{}\",\"content\":\"第二版\\n还有一行\"}}",
            note
        ),
    );
    assert!(again.ok, "{}", again.output);
    // 只读到一段 = 证据不足：整份覆盖被拒，并指出改法
    let mut obs2 = crate::capabilities::tools::api::Observations::default();
    let partial = run(
        &mut obs2,
        "read",
        format!("{{\"path\":\"{}\",\"limit\":1}}", note),
    );
    assert!(
        partial.ok && partial.output.contains("共 2 行"),
        "{}",
        partial.output
    );
    let refused = run(
        &mut obs2,
        "write",
        format!("{{\"path\":\"{}\",\"content\":\"覆盖\"}}", note),
    );
    assert!(
        !refused.ok && refused.output.contains("只读到一部分"),
        "{}",
        refused.output
    );
    assert!(
        refused.output.contains("edit"),
        "要给出改法：{}",
        refused.output
    );
    assert_eq!(
        io.get(&["demo", "work", "note.txt"]).as_deref(),
        Some("第二版\n还有一行"),
        "拒收时绝不写盘"
    );
    // 完整读过 → 放行
    let full = run(&mut obs2, "read", format!("{{\"path\":\"{}\"}}", note));
    assert!(
        full.ok && full.output.contains("已到文件末尾"),
        "{}",
        full.output
    );
    let ok = run(
        &mut obs2,
        "write",
        format!("{{\"path\":\"{}\",\"content\":\"第三版\"}}", note),
    );
    assert!(ok.ok, "{}", ok.output);
    // 读过之后文件被别人改过：指纹不符 → 拒绝凭记忆覆盖
    let mut obs3 = crate::capabilities::tools::api::Observations::default();
    run(&mut obs3, "read", format!("{{\"path\":\"{}\"}}", note));
    io.seed(&["demo", "work", "note.txt"], "别人改过的内容");
    let stale = run(
        &mut obs3,
        "write",
        format!("{{\"path\":\"{}\",\"content\":\"我的版本\"}}", note),
    );
    assert!(
        !stale.ok && stale.output.contains("又被改动过"),
        "{}",
        stale.output
    );
    assert_eq!(
        io.get(&["demo", "work", "note.txt"]).as_deref(),
        Some("别人改过的内容"),
        "拒不覆盖"
    );
}

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

#[test]
pub(crate) fn patch_channel_writes_files_without_json_escaping() {
    // 自由格式：信封之后原样跟补丁文本（含中文与换行，完全不转义）；一次两块、落在两个文件。
    let old = s(&["w", "m0", "note.txt"]);
    let new = s(&["w", "m0", "out.md"]);
    let body = format!(
        "*** Update File: {}\n*** SEARCH\n旧的第一行\n*** REPLACE\n新的第一行\n*** End File\n*** Add File: {}\n第一行\n第二行「带引号也没事」\n*** End File\n先改这两处。",
        old, new
    );
    let raw = format!("{{\"type\":\"tool\",\"name\":\"patch\"}}\n{}", body);
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "m0", "note.txt"], "旧的第一行\n第二行\n");
    let mut member = BTreeMap::new();
    member.insert(
        "m0".to_string(),
        vec![raw, "{\"type\":\"say\",\"text\":\"改好了\"}".to_string()],
    );
    let mut core = core_with_io_gateway(
        vec![module_of("m0")],
        gw(member, vec!["[]".into()]),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["m0"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "改一下", l)).unwrap();
    let views = tool_views(&events);
    assert!(views[0].ok, "两块都该成功：{}", views[0].output);
    assert!(
        views[0].output.contains("已应用 2 块改动"),
        "{}",
        views[0].output
    );
    // 补丁正文不上屏：它是工具输入，不是 AI 发言（自由格式工具只显示信封之前那段）
    let rows = transcript_rows(&events);
    assert!(
        rows.iter().all(|r| !r.1.contains("*** Update File")
            && !r.1.contains("*** Add File")
            && !r.1.contains("先改这两处")),
        "补丁正文与之后的散话都不该当成发言：{:?}",
        rows
    );
    assert_eq!(
        io.get(&["w", "m0", "note.txt"]).as_deref(),
        Some("新的第一行\n第二行\n"),
        "只改 SEARCH 指定的那几行"
    );
    // 原样落盘：模型写了几行就是几行（末尾没有空行就不补——与 read/write 的"照原文"口径一致）
    assert_eq!(
        io.get(&["w", "m0", "out.md"]).as_deref(),
        Some("第一行\n第二行「带引号也没事」"),
        "新建文件的整份内容原样落盘（补丁之后那句话没有混进去）"
    );
}

#[test]
pub(crate) fn a_failing_patch_block_writes_nothing_at_all() {
    // 第 2 块找不到 SEARCH：整体不写盘，回执点名第几块、为什么
    let first = s(&["w", "m0", "a.txt"]);
    let second = s(&["w", "m0", "b.txt"]);
    let body = format!(
        "*** Add File: {}\n新文件内容\n*** End File\n*** Update File: {}\n*** SEARCH\n这行不存在\n*** REPLACE\nx\n*** End File\n",
        first, second
    );
    let raw = format!("{{\"type\":\"tool\",\"name\":\"patch\"}}\n{}", body);
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "m0", "b.txt"], "只有这一行\n");
    let mut member = BTreeMap::new();
    member.insert(
        "m0".to_string(),
        vec![raw, "{\"type\":\"say\",\"text\":\"知道了\"}".to_string()],
    );
    let mut core = core_with_io_gateway(
        vec![module_of("m0")],
        gw(member, vec!["[]".into()]),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["m0"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "改两处", l)).unwrap();
    let views = tool_views(&events);
    assert!(!views[0].ok, "{}", views[0].output);
    assert!(views[0].output.contains("第 2 块"), "{}", views[0].output);
    assert!(
        views[0].output.contains("没有任何文件被写入"),
        "{}",
        views[0].output
    );
    assert!(views[0].output.contains("找不到"), "{}", views[0].output);
    assert_eq!(
        io.get(&["w", "m0", "a.txt"]),
        None,
        "第 1 块也不许写盘（原子）"
    );
    assert_eq!(
        io.get(&["w", "m0", "b.txt"]).as_deref(),
        Some("只有这一行\n"),
        "没改"
    );
}

#[test]
pub(crate) fn a_patch_without_end_marker_is_refused_with_the_line() {
    // 少了 End File：不能猜哪里是结尾（否则补丁后面那句话会被写进文件）
    let target = s(&["w", "m0", "out.md"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"name\":\"patch\"}}\n*** Add File: {}\n内容\n我改完了。\n",
        target
    );
    let io = Arc::new(InMemorySysIo::new());
    let mut member = BTreeMap::new();
    member.insert(
        "m0".to_string(),
        vec![raw, "{\"type\":\"say\",\"text\":\"知道了\"}".to_string()],
    );
    let mut core = core_with_io_gateway(
        vec![module_of("m0")],
        gw(member, vec!["[]".into()]),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["m0"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "加个文件", l)).unwrap();
    let views = tool_views(&events);
    assert!(
        !views[0].ok && views[0].output.contains("*** End File"),
        "{}",
        views[0].output
    );
    assert_eq!(io.get(&["w", "m0", "out.md"]), None, "绝不落盘");
}

#[test]
pub(crate) fn rewind_clears_the_read_ledger_so_overwrite_needs_a_fresh_read() {
    // 回档把转录截掉了：那段"我完整读过 / 我写过"的证据随之作废（保守，宁肯让模型重读）。
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["w", "a", "note.txt"]);
    let write = format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"v1\"}}}}",
        note
    );
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            write.clone(),
            "{\"type\":\"say\",\"text\":\"写好了\"}".to_string(),
            write.clone(),
            "{\"type\":\"say\",\"text\":\"又写了一次\"}".to_string(),
        ],
    );
    let mut core = core_with_io(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    // 第一轮：新建，放行
    let e1 = with_live(|l| core.single_say(&sid, "写", l)).unwrap();
    let v1 = tool_views(&e1);
    assert!(v1[0].ok, "新建文件不需要先读过：{}", v1[0].output);
    // 回档：证据作废
    core.rewind(&sid, 0).unwrap();
    // 第二轮：同一个路径已存在，而账本已被清空 → 拒绝并提示先读
    let e2 = with_live(|l| core.single_say(&sid, "再写", l)).unwrap();
    let v2 = tool_views(&e2);
    assert!(!v2[0].ok, "回档后旧的读取证据不再算数：{}", v2[0].output);
    assert!(
        v2[0].output.contains("必须在本次会话里先"),
        "{}",
        v2[0].output
    );
    assert_eq!(
        io.get(&["w", "a", "note.txt"]).as_deref(),
        Some("v1"),
        "拒不覆盖"
    );
}

#[test]
pub(crate) fn builtin_read_reports_errors_verbatim() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(&["demo", "work", "note.txt"], "内容");
    let note = s(&["demo", "work", "note.txt"]);
    let ok = run_builtin(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", note));
    assert!(ok.ok && ok.output.contains("内容"), "{}", ok.output);
    assert!(
        ok.output.contains(&note),
        "回执要写明读的是哪个文件：{}",
        ok.output
    );
    let missing = run_builtin(
        &sb,
        &io,
        "read",
        &format!("{{\"path\":\"{}\"}}", s(&["demo", "work", "nope.txt"])),
    );
    assert!(!missing.ok);
    assert!(missing.output.contains("不存在"), "{}", missing.output);
    // 参数不合法 = 如实报错，不猜用户想干什么。
    assert!(!run_builtin(&sb, &io, "read", "{").ok);
    assert!(!run_builtin(&sb, &io, "read", "{}").ok);
    // 非绝对路径一律拒绝（相对路径、带冒号前缀的伪路径都落在这里）。
    let rel = run_builtin(&sb, &io, "read", "{\"path\":\"nope.txt\"}");
    assert!(
        !rel.ok && rel.output.contains("需要绝对路径"),
        "{}",
        rel.output
    );
    let fake = run_builtin(&sb, &io, "read", "{\"path\":\"work:/nope.txt\"}");
    assert!(
        !fake.ok && fake.output.contains("需要绝对路径"),
        "{}",
        fake.output
    );
}

/// list：列目录（名字 / 大小，按名字排序）；read 遇到目录**如实引导**到 list，而不是抛 IO 错。
#[test]
pub(crate) fn builtin_list_shows_a_directory_and_read_guides_to_it() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(&["demo", "work", "b.txt"], "bb");
    io.seed(&["demo", "work", "a.md"], "aaaa");
    let work = s(&["demo", "work"]);

    let ls = run_builtin(&sb, &io, "list", &format!("{{\"path\":\"{}\"}}", work));
    assert!(ls.ok, "{}", ls.output);
    assert!(
        ls.output.contains("a.md") && ls.output.contains("b.txt"),
        "{}",
        ls.output
    );
    assert!(ls.output.contains("2 项"), "{}", ls.output);
    let a = ls.output.find("a.md").expect("a.md");
    let b = ls.output.find("b.txt").expect("b.txt");
    assert!(a < b, "该按名字排序：{}", ls.output);

    // read 一个目录：给引导，不给 IO 错。
    let rd = run_builtin(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", work));
    assert!(!rd.ok, "read 目录不该成功：{}", rd.output);
    assert!(
        rd.output.contains("用 list"),
        "该引导到 list：{}",
        rd.output
    );
}

#[test]
pub(crate) fn builtin_read_range_numbers_lines_and_points_at_the_next_offset() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(&["demo", "work", "note.txt"], "l1\nl2\nl3\nl4\nl5\n");
    let note = s(&["demo", "work", "note.txt"]);
    let read = |args: &str| {
        run_builtin(
            &sb,
            &io,
            "read",
            &format!("{{\"path\":\"{}\",{}}}", note, args),
        )
    };
    // 整读：行号从 1 数起，末尾如实说共几行
    let all = read("\"offset\":1");
    assert!(all.ok, "{}", all.output);
    assert!(
        all.output.contains("1: l1") && all.output.contains("5: l5"),
        "{}",
        all.output
    );
    assert!(
        all.output.contains("已到文件末尾，共 5 行"),
        "{}",
        all.output
    );
    // 区间读：只给这一段，并给出接着读的 offset
    let mid = read("\"offset\":2,\"limit\":2");
    assert!(
        mid.output.contains("2: l2") && mid.output.contains("3: l3"),
        "{}",
        mid.output
    );
    assert!(
        !mid.output.contains("1: l1") && !mid.output.contains("4: l4"),
        "不该越出请求的区间：{}",
        mid.output
    );
    assert!(
        mid.output
            .contains("已显示第 2-3 行，共 5 行；继续读用 offset=4"),
        "{}",
        mid.output
    );
    // 末尾区间：到文件末尾
    let last = read("\"offset\":5,\"limit\":2");
    assert!(
        last.output.contains("5: l5") && last.output.contains("已到文件末尾，共 5 行"),
        "{}",
        last.output
    );
    // 越过末行：不是错误，如实说总行数
    let past = read("\"offset\":9");
    assert!(past.ok, "越过末行要如实告知而不是报错：{}", past.output);
    assert!(
        past.output.contains("超出末行：该文件共 5 行"),
        "{}",
        past.output
    );
}

#[test]
pub(crate) fn builtin_arg_mistakes_are_named_and_the_signature_comes_back() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(&["demo", "work", "note.txt"], "内容\n");
    let note = s(&["demo", "work", "note.txt"]);
    let run = |tool: &str, args: &str| run_builtin(&sb, &io, tool, args);
    // 上界由声明给出（不再是代码里的手写判断）
    let big = run("read", &format!("{{\"path\":\"{}\",\"limit\":3000}}", note));
    assert!(
        !big.ok && big.output.contains("参数 limit 不能大于 2000"),
        "{}",
        big.output
    );
    assert!(
        big.output.contains("read\n读取文本文件（UTF-8）。"),
        "失败要把工具签名发回去：{}",
        big.output
    );
    assert!(
        big.output
            .contains("- limit（integer，缺省 2000，不小于 1，不大于 2000）"),
        "{}",
        big.output
    );
    let small = run("read", &format!("{{\"path\":\"{}\",\"offset\":0}}", note));
    assert!(
        !small.ok && small.output.contains("参数 offset 不能小于 1"),
        "{}",
        small.output
    );
    let wrong = run("read", &format!("{{\"path\":{}}}", 1));
    assert!(
        !wrong.ok && wrong.output.contains("参数 path 需要 string"),
        "{}",
        wrong.output
    );
    let unknown = run(
        "read",
        &format!("{{\"path\":\"{}\",\"encoding\":\"utf8\"}}", note),
    );
    assert!(
        !unknown.ok && unknown.output.contains("没有参数 encoding"),
        "{}",
        unknown.output
    );
    let missing = run("write", &format!("{{\"path\":\"{}\"}}", note));
    assert!(
        !missing.ok && missing.output.contains("缺少必填参数 content"),
        "{}",
        missing.output
    );
    let empty = run(
        "search",
        &format!("{{\"path\":\"{}\",\"keyword\":\"\"}}", note),
    );
    assert!(
        !empty.ok && empty.output.contains("参数 keyword 不能是空字符串"),
        "{}",
        empty.output
    );
    let not_object = run("read", "\"just a string\"");
    assert!(
        !not_object.ok && not_object.output.contains("args 必须是一个参数对象"),
        "{}",
        not_object.output
    );
    let nope = run("nope", "{}");
    assert!(
        !nope.ok && nope.output.contains("未知的内置工具：nope"),
        "{}",
        nope.output
    );
}

#[test]
pub(crate) fn builtin_tool_book_is_the_one_source_of_names_and_paths() {
    // 保留名（代码里的常量）**必须都有声明**，否则模型看到的工具与放行的工具会走偏。
    // 反过来不成立：总表里还有协作动词（say/agree/leave/ask），它们的实现不在 systool。
    let prompts = test_prompts();
    let systools = test_systools();
    let book = &systools.tools;
    for name in crate::capabilities::tools::api::names() {
        assert!(
            book.contains_key(&name),
            "保留名 {} 必须在工具总表里有声明",
            name
        );
    }
    // 协作动词也在总表里，且不碰文件系统（capability = none）。
    for verb in ["say", "agree", "leave", "ask"] {
        let schema = book
            .get(verb)
            .unwrap_or_else(|| panic!("动词 {} 该在总表里", verb));
        assert_eq!(schema.capability, "none", "{} 不碰文件系统", verb);
    }
    // 内置工具一律按真实绝对路径寻址：JSON 工具必须声明必填 path；自由格式工具（patch）不吃参数校验。
    for (name, schema) in book {
        // 只查文件域工具（按真实绝对路径寻址）：协作动词不碰文件系统，不吃这条。
        if schema.capability == "none" {
            continue;
        }
        if crate::capabilities::tools::api::is_freeform(name) {
            assert!(
                schema.params.is_none(),
                "自由格式工具不声明 JSON 参数：{}",
                name
            );
            continue;
        }
        let path = schema.params.as_ref().and_then(|p| p.get("path"));
        assert!(
            path.map(|p| p.required).unwrap_or(false),
            "{} 必须声明必填 path",
            name
        );
    }
    // 工作环境块：**只有路径与规矩，没有工具清单**（总表只留在核心手里当判据）。
    let sb = test_sandbox("a1", &[]);
    let env = crate::capabilities::session::domain::session::env_block(
        &prompts,
        &crate::capabilities::session::api::SessionParams::from_workspace("a1", &sb, &[]),
    );
    assert!(env.contains("【工作环境】"), "{}", env);
    assert!(
        !env.contains("【工具参数】") && !env.contains("offset（integer"),
        "系统提示里不许出现工具清单：{}",
        env
    );

    // 工具说明来自同一份声明，但**按回合注入**：核心查这一回合的身份，只渲染它那一份。
    let m = member_with_tools("a1", vec![], Arc::new(SilentRunner));
    let block = m
        .tools
        .as_ref()
        .expect("工具环境")
        .tools_block(&crate::capabilities::tools::api::names(), true);
    assert!(block.contains("【本回合可用的工具】"), "{}", block);
    assert!(
        block.contains("- offset（integer，缺省 1，不小于 1）"),
        "{}",
        block
    );
    assert!(
        block.contains("- ignore_case（boolean）：是否忽略大小写；省略即区分大小写"),
        "{}",
        block
    );
    // patch 的写法说明跟着它一起注入（自由格式：正文不走 JSON）
    assert!(block.contains("【改文件：用 patch"), "{}", block);
    assert!(
        block.contains("*** End File"),
        "每块要收尾这件事必须写清楚：{}",
        block
    );
    // 没有拿到的工具**不进**这个块（这是"总表不进提示词"的正面表述）。
    let only_read = m
        .tools
        .as_ref()
        .expect("工具环境")
        .tools_block(&["read".to_string()], false);
    assert!(only_read.contains("- offset（integer"), "{}", only_read);
    assert!(!only_read.contains("patch"), "{}", only_read);
}
