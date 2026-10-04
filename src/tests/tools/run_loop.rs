//! 工具循环与内置工具：循环、无调用上限、读写的“读过”证据
use super::super::builders::*;
use super::super::prelude::*;
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
        &[crate::capabilities::prompt::api::Segment::MechanismCollab],
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
