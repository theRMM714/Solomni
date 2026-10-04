//! 补丁通道、回档后的账本与内置工具的边界（错误如实）
use super::super::builders::*;
use super::super::prelude::*;
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
        &crate::capabilities::session::api::SessionParams::from_workspace(
            "a1",
            &sb,
            &[],
            vec![crate::capabilities::prompt::api::Segment::MechanismCollab],
        ),
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
