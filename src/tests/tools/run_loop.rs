//! 目的：钉工具循环无调用上限，与内置工具「读过」的证据链。
//! 管：清单参数声明与执行、拒绝未声明/书外工具、无调用上限、grep 行号与大小写、直接会话文件工具、原生模式失败行、模块目录写拒绝、edit 范围与整读要求。
//! 不管：工具面发放（face.rs）。
//! 联动：src/units/tools/、session 事实与账本。

use super::super::builders::*;
use super::super::prelude::*;
use crate::units::conversation::session::api::SINGLE;
use crate::units::conversation::session::api::run_batch;
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

/// 目的：系统工具册里、但不在本回合工具面的工具（如 compact）必须按越权拒绝，不能落进模块工具兜底。
#[test]
pub(crate) fn tool_loop_refuses_book_only_tools_not_in_the_face() {
    let mut m = member_with_tools(
        "m0",
        vec![
            "{\"type\":\"tool\",\"name\":\"compact\",\"args\":{}}".into(),
            "{\"type\":\"say\",\"text\":\"完成\"}".into(),
        ],
        Arc::new(SilentRunner),
    );
    let prompts = test_prompts();
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &prompts);
    let trace = exec.traces.get("m0").expect("工具调用应入册");
    assert!(
        trace.iter().any(|v| v.output.contains("没有工具 compact")),
        "册里但不在工具面的工具该按越权拒绝：{:?}",
        trace.iter().map(|v| &v.output).collect::<Vec<_>>()
    );
    assert!(
        !trace.iter().any(|v| v.output.contains("缺少必填参数")),
        "越权拒绝不该先走参数校验：{:?}",
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
pub(crate) fn builtin_grep_reports_line_numbers_and_respects_case() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    io.seed(
        &["demo", "a1", "note.txt"],
        "第一行 Alpha\n第二行 beta\nalpha 小写\n",
    );
    let path = s(&["demo", "a1", "note.txt"]);
    // 默认区分大小写
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!("{{\"path\":\"{}\",\"pattern\":\"alpha\"}}", path),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains(":3:alpha 小写"), "{}", out.output);
    assert!(
        !out.output.contains("第一行 Alpha"),
        "默认区分大小写：{}",
        out.output
    );
    assert!(
        out.output.contains("命中 1 行 / 1 个文件"),
        "{}",
        out.output
    );
    // ignore_case = true
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"alpha\",\"ignore_case\":true}}",
            path
        ),
    );
    assert!(
        out.output.contains(":1:第一行 Alpha") && out.output.contains(":3:alpha 小写"),
        "{}",
        out.output
    );
    assert!(
        out.output.contains("命中 2 行 / 1 个文件"),
        "{}",
        out.output
    );
    // 越界被拒
    let bad = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"x\"}}",
            s(&["outside", "f.txt"])
        ),
    );
    assert!(!bad.ok && bad.output.contains("路径越界"), "{}", bad.output);
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
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "记一笔",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
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

/// 目的：信封通道里供应商返回原生 tool_calls 时**不执行**，但要落一条失败工具行 + 日志，
///   不能静默丢成“正文停在冒号”（真实会话里用户看到的就是这个）。
#[test]
pub(crate) fn envelope_mode_native_calls_become_a_failed_tool_line() {
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["w", "a", "note.txt"]);
    let gateway = NativeCallGateway {
        name: "read".to_string(),
        args_json: serde_json::json!({ "path": note }).to_string(),
        raw: "先确认一下：".to_string(),
    };
    let mut core = core_with_io_gateway(vec![module_of("a")], gateway, Arc::clone(&io));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "继续",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let views: Vec<_> = events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => {
                ls.iter().filter_map(|l| l.tool.clone()).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(views.len(), 1, "{:?}", views);
    assert_eq!(views[0].name, "read");
    assert!(!views[0].ok, "通道错位不能算成功");
    assert!(
        views[0].output.contains("原生工具调用"),
        "必须把通道错位说清楚：{}",
        views[0].output
    );
    assert!(
        !views[0].output.contains("你好"),
        "没有执行就不该有读取结果：{}",
        views[0].output
    );
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("原生工具调用")),
        "失败工具行也要回注上下文，模型才知道换信封重发"
    );
}

/// 目的：模块目录（含 `userdata/`）对 agent **永久只读**：内置 write 一律拒绝，不落盘。
#[test]
pub(crate) fn builtin_write_into_module_dir_is_denied() {
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &["data"]);
    let cases: [&[&str]; 2] = [
        &["mods", "data", "keep.txt"],
        &["mods", "data", "userdata", "state.json"],
    ];
    for rel in cases {
        let path = s(rel);
        let out = run_builtin(
            &sb,
            &io,
            "write",
            &format!("{{\"path\":\"{}\",\"content\":\"状态\"}}", path),
        );
        assert!(!out.ok, "模块目录对 agent 只读：{}", out.output);
        assert!(
            out.output.contains("模块 data"),
            "拒绝理由要说清是模块目录：{}",
            out.output
        );
        assert_eq!(io.get(rel), None, "不得落盘");
    }
    // 别人的模块（不在本席模块表里）：同样拒绝且不落盘。
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
    let note = s(&["demo", "a1", "note.txt"]);
    io.seed(&["demo", "a1", "note.txt"], "第一段\n要改的句子\n第三段\n");
    let mut obs = crate::units::tools::api::Observations::default();
    let exec = test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::clone(&io),
        Arc::new(NoFenceHost),
    );
    let edit = |obs: &mut crate::units::tools::api::Observations, args: &str| {
        let full = format!("{{\"path\":\"{}\",{}}}", note, args);
        let done = exec.run_builtin(
            &sb,
            &test_systools().tools,
            std::mem::take(obs),
            None,
            crate::units::tools::api::ResultBudget::full(),
            "edit",
            &full,
        );
        *obs = done.observations;
        done.outcome
    };
    // 唯一命中：只改那一处，别处一字不动
    let ok = edit(
        &mut obs,
        "\"old_string\":\"要改的句子\",\"new_string\":\"改好了\"",
    );
    assert!(ok.ok, "{}", ok.output);
    assert_eq!(
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
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
    io.seed(&["demo", "a1", "note.txt"], "dup\ndup\n");
    let multi = edit(&mut obs, "\"old_string\":\"dup\",\"new_string\":\"x\"");
    assert!(
        !multi.ok && multi.output.contains("命中 2 处") && multi.output.contains("第 1、2 行"),
        "{}",
        multi.output
    );
    assert_eq!(
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
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
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
        Some("x\nx\n")
    );
    // new_string 空串 = 把这一段删掉
    let del = edit(
        &mut obs,
        "\"old_string\":\"x\",\"new_string\":\"\",\"replace_all\":true",
    );
    assert!(del.ok, "{}", del.output);
    assert_eq!(io.get(&["demo", "a1", "note.txt"]).as_deref(), Some("\n\n"));
}

#[test]
pub(crate) fn builtin_edit_refuses_files_it_cannot_see_whole() {
    // 只看到开头 / 看到的是替换字符：改写会把没读到的内容或原始字节一起弄丢 → 一律不写盘。
    let note = s(&["demo", "a1", "note.txt"]);
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
        io.seed(&["demo", "a1", "note.txt"], "abc");
        let mut obs = crate::units::tools::api::Observations::default();
        let exec = test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::clone(&io),
            Arc::new(NoFenceHost),
        );
        let done = exec.run_builtin(
            &sb,
            &test_systools().tools,
            std::mem::take(&mut obs),
            None,
            crate::units::tools::api::ResultBudget::full(),
            "edit",
            &args,
        );
        obs = done.observations;
        let out = done.outcome;
        assert!(!out.ok && out.output.contains(want), "{}", out.output);
        assert_eq!(
            io.get(&["demo", "a1", "note.txt"]).as_deref(),
            Some("abc"),
            "拒绝时不写盘"
        );
    }
}

#[test]
pub(crate) fn builtin_write_needs_a_complete_prior_read_of_an_existing_file() {
    let io = Arc::new(InMemorySysIo::new());
    let sb = test_sandbox("a1", &[]);
    let note = s(&["demo", "a1", "note.txt"]);
    let exec = test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::clone(&io),
        Arc::new(NoFenceHost),
    );
    let run = |obs: &mut crate::units::tools::api::Observations, tool: &str, args: String| {
        let done = exec.run_builtin(
            &sb,
            &test_systools().tools,
            std::mem::take(obs),
            None,
            crate::units::tools::api::ResultBudget::full(),
            tool,
            &args,
        );
        *obs = done.observations;
        done.outcome
    };
    let mut obs = crate::units::tools::api::Observations::default();
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
    let mut obs2 = crate::units::tools::api::Observations::default();
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
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
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
    let mut obs3 = crate::units::tools::api::Observations::default();
    run(&mut obs3, "read", format!("{{\"path\":\"{}\"}}", note));
    io.seed(&["demo", "a1", "note.txt"], "别人改过的内容");
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
        io.get(&["demo", "a1", "note.txt"]).as_deref(),
        Some("别人改过的内容"),
        "拒不覆盖"
    );
}

/// 目的：剩余上下文连最小回执都放不下时，run_batch 必须在执行前整批失败——shell 绝不产生副作用。
#[test]
pub(crate) fn exhausted_budget_skips_the_whole_batch() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "不该跑".into(),
        ok: true,
    });
    let mut wt = member_with_tools("m0", Vec::new(), Arc::clone(&runner));
    let mut tools = wt.tools.take().expect("工具面");
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let plan = vec![
        (
            Some("m0".to_string()),
            "scan".to_string(),
            "{\"pattern\":\"x\"}".to_string(),
        ),
        (None, "read".to_string(), "{\"path\":\"a\"}".to_string()),
    ];
    let budget = crate::units::tools::api::ResultBudget::for_batch(20_000, 19_800, 2);
    assert!(budget.exhausted(), "构造前提：预算已枯竭");
    let done = run_batch(&mut tools, &plan, None, &cancel, budget);
    assert_eq!(done.len(), 2, "每个调用都要有回执");
    for (_, outcome, _, _) in &done {
        assert!(!outcome.ok);
        assert!(
            outcome.output.contains("上下文剩余不足") && outcome.output.contains("没有执行"),
            "{}",
            outcome.output
        );
    }
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "预检失败不得落进程"
    );
}

/// 目的：read 的可见内容与观察事实必须同受一批预算约束；被预算切掉的行绝不能记成读过。
#[test]
pub(crate) fn read_respects_the_batch_budget_and_records_only_what_it_shows() {
    let io = Arc::new(InMemorySysIo::new());
    let sb = test_sandbox("a1", &[]);
    let path = s(&["demo", "a1", "long.txt"]);
    let long = "a".repeat(2_500);
    io.seed(
        &["demo", "a1", "long.txt"],
        &format!("{}\nsecond\nthird\n", long),
    );
    let exec = test_tools_svc_with(
        Arc::new(SilentRunner),
        Arc::clone(&io),
        Arc::new(NoFenceHost),
    );
    let budget = crate::units::tools::api::ResultBudget::for_batch(20_000, 17_500, 1);
    assert_eq!(budget.limit(), 1_000, "构造前提：这一批只给 1000 字符");
    assert!(!budget.exhausted(), "构造前提：还没枯竭，read 应当执行");
    let done = exec.run_builtin(
        &sb,
        &test_systools().tools,
        crate::units::tools::api::Observations::default(),
        None,
        budget,
        "read",
        &format!("{{\"path\":\"{}\"}}", path),
    );
    assert!(done.outcome.ok, "{}", done.outcome.output);
    assert!(
        done.outcome.output.chars().count() <= budget.limit(),
        "read 自己就得收进预算，不能留给 round 二次截断：{} 字符",
        done.outcome.output.chars().count()
    );
    assert!(
        done.outcome.output.contains("本行超过本次预算"),
        "第一行超预算要如实标记：{}",
        done.outcome.output
    );
    assert!(
        done.outcome
            .notes
            .iter()
            .any(|n| n.code == crate::kernel::api::note::codes::RESULT_TRUNCATED_CONTEXT),
        "read 因预算少给行时必须提示先压缩会话：{:?}",
        done.outcome.notes
    );
    assert!(
        done.observations.facts().is_empty(),
        "显示被截断的读取不得留下观察事实"
    );
}

#[test]
pub(crate) fn module_and_service_params_share_one_header() {
    let mut m = module_of("m0");
    m.manifest.tools.insert(
        "scan".to_string(),
        decl_with(
            "python tools/scan.py",
            "keyword: {type: string, required: true}\n",
        ),
    );
    let service_sections = vec!["- m0.svc.echo\nx（integer，必填）".to_string()];
    let notes = crate::units::tools::api::tool_notes(
        &test_prompts(),
        &test_sandbox("m0", &["m0"]),
        std::slice::from_ref(&m),
        &service_sections,
    );
    let block = &notes.module_tool_params;
    assert_eq!(
        block.matches("【模块工具参数】").count(),
        1,
        "静态模块参数与服务参数只能共用一个标题：{}",
        block
    );
    assert!(block.contains("m0.scan"), "{}", block);
    assert!(block.contains("m0.svc.echo"), "{}", block);
    assert!(
        !block.contains("都没声明参数"),
        "有参数时不能出现空态文案：{}",
        block
    );

    // 只有服务参数时同样只出现一个标题，也不能退回"没声明参数"的空态。
    let only_service = crate::units::tools::api::tool_notes(
        &test_prompts(),
        &test_sandbox("m0", &["m0"]),
        &[],
        &service_sections,
    );
    assert_eq!(
        only_service
            .module_tool_params
            .matches("【模块工具参数】")
            .count(),
        1,
        "{}",
        only_service.module_tool_params
    );
    assert!(
        !only_service.module_tool_params.contains("都没声明参数"),
        "{}",
        only_service.module_tool_params
    );
}

#[test]
pub(crate) fn service_ops_are_scoped_to_the_agent_module_face() {
    use crate::units::conversation::session::api::{ServiceOp, service_ops_for_modules};
    let op = |tool: &str| ServiceOp {
        tool: tool.to_string(),
        service: "svc".to_string(),
        op: tool.to_string(),
        description: String::new(),
        parameters: None,
    };
    let started = BTreeMap::from([
        ("m0".to_string(), vec![op("svc.echo")]),
        ("other".to_string(), vec![op("svc.echo")]),
    ]);
    let filtered = service_ops_for_modules(started, &[module_of("m0")]);
    assert_eq!(
        filtered.keys().cloned().collect::<Vec<_>>(),
        vec!["m0".to_string()],
        "未分配模块的服务不能进提示词"
    );
    assert_eq!(filtered["m0"].len(), 1);
}

/// 目的：同一批里的 read 不授权同批的 write；下一批才生效。
/// 约束：账本取**批前快照**，读结果要等这一批回填进转录后才算模型看过。
#[test]
pub(crate) fn read_in_the_same_batch_does_not_authorize_write() {
    use crate::units::llm::api::ToolCall;
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "base.md"], "base\n");
    let path = abs(&["w", "a", "base.md"]);
    let ws = test_workspace_store(
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(InMemoryWorkStore::new()),
    );
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"base\n", 1, 0)
        .unwrap();
    let write = || ToolCall {
        id: "write".to_string(),
        name: "write".to_string(),
        args_json: serde_json::json!({"path": path, "content": "new\n"}).to_string(),
    };
    let read = || ToolCall {
        id: "read".to_string(),
        name: "read".to_string(),
        args_json: serde_json::json!({"path": path}).to_string(),
    };
    let declared = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut m = native_member(
        "a",
        Arc::clone(&io),
        vec![
            NativeStep::Calls(vec![read(), write()]),
            NativeStep::Calls(vec![read(), write()]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"完成\"}".to_string()),
        ],
        declared,
        seen,
    );
    {
        let t = m.tools.as_mut().expect("工具环境");
        let mut sb = t.face.sandbox.clone();
        sb.work_name = "w".to_string();
        sb.session = "w".to_string();
        sb.shared = abs(&["w", "work"]);
        sb.private = abs(&["w", "a"]);
        t.face.sandbox = sb.clone();
        t.face.fence = crate::kernel::api::fence::FencePlan::for_session(&sb, &[], false);
        t.workspace = Some(ws);
    }
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &test_prompts());
    let trace = exec.traces.get("a").expect("工具记录");
    let writes: Vec<&crate::units::transcript::api::ToolCallView> =
        trace.iter().filter(|v| v.name == "write").collect();
    assert_eq!(writes.len(), 2, "两批各写一次：{trace:?}");
    assert!(
        !writes[0].ok && writes[0].output.contains("FS_NOT_OBSERVED"),
        "同批 read 不能授权 write：{:?}",
        writes[0].output
    );
    assert!(
        writes[1].ok,
        "上一批的 read 已进转录，下一次 write 应放行：{:?}",
        writes[1].output
    );
}

/// 目的：work_commit 是发布口——把没读过的受管改动提交上去等于绕过 write 门禁，必须硬拒。
/// 约束：只拦 tracked 且当前内容与基线不同的路径；新文件与删除不在这一道门里。
#[test]
pub(crate) fn work_commit_refuses_unobserved_tracked_changes() {
    use crate::units::llm::api::ToolCall;
    let io = Arc::new(InMemorySysIo::new());
    let store = Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(Arc::new(InMemoryWorkspace::new()), store.clone());
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "base.md", b"base\n", 1, 0)
        .unwrap();
    // 沙箱里改过、但模型没读过：账本为空，commit 必须拒绝。
    store.seed(&abs(&["w", "a"]), "base.md", "changed\n");
    let commit = ToolCall {
        id: "commit".to_string(),
        name: "work_commit".to_string(),
        args_json: serde_json::json!({"paths": ["base.md"], "message": "未读提交"}).to_string(),
    };
    let mut m = native_member(
        "a",
        Arc::clone(&io),
        vec![
            NativeStep::Calls(vec![commit]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"完成\"}".to_string()),
        ],
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    {
        let t = m.tools.as_mut().expect("工具环境");
        let mut sb = t.face.sandbox.clone();
        sb.work_name = "w".to_string();
        sb.session = "w".to_string();
        sb.shared = abs(&["w", "work"]);
        sb.private = abs(&["w", "a"]);
        t.face.sandbox = sb.clone();
        t.face.fence = crate::kernel::api::fence::FencePlan::for_session(&sb, &[], false);
        t.handlers =
            crate::units::workspace::api::work_handlers(&ws, &sb, &["work_commit".to_string()]);
        t.face.allowed.push("work_commit".to_string());
        t.workspace = Some(Arc::clone(&ws));
    }
    let exec = run_execution(std::slice::from_mut(&mut m), "任务", &test_prompts());
    let trace = exec.traces.get("a").expect("工具记录");
    let commits: Vec<&crate::units::transcript::api::ToolCallView> =
        trace.iter().filter(|v| v.name == "work_commit").collect();
    assert_eq!(commits.len(), 1, "一次提交：{trace:?}");
    assert!(
        !commits[0].ok && commits[0].output.contains("FS_NOT_OBSERVED"),
        "未读的受管改动不能经 work_commit 发布：{:?}",
        commits[0].output
    );
}
