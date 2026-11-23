//! 目的：钉 llm 能力的信封解析/修复与模型目录、端点回落。
//! 管：干净/降级/包裹 JSON 解析、文本可缺、配平提取、坏信封修复与分类、工具解析、流式在信封括号停止、抑制分片、目录与端点候选。
//! 不管：前端路由（routes.rs）。
//! 联动：src/capabilities/llm/。

use super::builders::*;
use super::prelude::*;
use crate::capabilities::conversation::session::api::SINGLE;

#[test]
pub(crate) fn envelope_parse_clean() {
    let r = crate::capabilities::llm::api::parse("{\"type\":\"ask\",\"text\":\"要 A 还是 B？\"}");
    assert!(matches!(r.verb, crate::capabilities::llm::api::Verb::Ask));
    assert_eq!(r.text, "要 A 还是 B？");
    assert!(!r.degraded);
}

#[test]
pub(crate) fn envelope_degraded_keeps_raw() {
    let r = crate::capabilities::llm::api::parse("这不是 JSON");
    assert!(r.degraded);
    assert_eq!(r.text, "这不是 JSON");
}

#[test]
pub(crate) fn envelope_wrapped_json_still_parses() {
    let r =
        crate::capabilities::llm::api::parse("好的：{\"type\":\"agree\",\"text\":\"同意\"} 以上。");
    assert!(matches!(r.verb, crate::capabilities::llm::api::Verb::Agree));
    assert!(!r.degraded);
}

#[test]
pub(crate) fn envelope_text_may_be_omitted() {
    let r = crate::capabilities::llm::api::parse("{\"type\":\"agree\"}");
    assert!(
        matches!(r.verb, crate::capabilities::llm::api::Verb::Agree),
        "缺 text 不影响表态"
    );
    assert_eq!(r.text, "", "缺 text = 空串");
    assert!(!r.degraded);
    let say = crate::capabilities::llm::api::parse("{\"type\":\"say\"}");
    assert!(
        matches!(say.verb, crate::capabilities::llm::api::Verb::Say) && !say.degraded,
        "缺 text 的发言仍是干净信封"
    );
    assert!(say.text.is_empty());
    // 缺 name 的工具信封仍是 malformed 信号，不被缺省 text 收编成发言。
    let bad = crate::capabilities::llm::api::parse("{\"type\":\"tool\",\"args\":{}}");
    assert!(matches!(
        bad.verb,
        crate::capabilities::llm::api::Verb::Tool
    ));
    assert!(bad.tools.iter().any(|t| t.malformed.is_some()));
}

// ---------- 工具执行器 ----------
#[test]
pub(crate) fn malformed_tool_envelope_becomes_a_failed_tool_line() {
    // 真实案例：模型想调 write，但信封 JSON 非法（结尾多个 ]）——
    // 必须记一条 ok=false 的 tool 行，绝不把 JSON 当 AI 消息渲染，也绝不执行工具。
    let raw_path = s(&["w", "work", "README.md"]);
    let broken = broken_tool(&raw_path);
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            broken.clone(),
            "{\"type\":\"say\",\"text\":\"改好了\"}".to_string(),
        ],
    );
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "写个 README",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        1,
        "应有一条 tool 行：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 绝不能当文本渲染：{:?}",
        rows
    );
    let views = tool_views(&events);
    assert_eq!(views.len(), 1);
    assert!(!views[0].ok, "非法的调用必须记为失败");
    assert_eq!(views[0].name, "write", "名字要尽力打捞出来（只用于显示）");
    assert_eq!(views[0].module, "", "没写 module → 空串");
    assert!(
        views[0].output.contains("不是合法 JSON"),
        "回注册子文案：{}",
        views[0].output
    );
    assert_eq!(
        transcript_raws(&events),
        vec![broken.clone()],
        "原文留档（重建上下文用；每条回复一份）"
    );
    assert_eq!(
        io.get(&["w", "work", "README.md"]),
        None,
        "非法信封绝不执行工具"
    );
    // 历史：assistant(原文) + [工具结果]（含册子文案）→ 模型下一轮能自己改
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter()
            .any(|m| m.role == "assistant" && m.content == broken)
    );
    assert!(
        h.iter().any(|m| m.role == "user"
            && m.content.contains("[工具结果] write")
            && m.content.contains("不是合法 JSON")),
        "{:?}",
        h
    );
    // 重建一致（重启后从落盘流水重建上下文）
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    crate::tests::lifecycle::rewind(
        &mut core2,
        &sid,
        crate::capabilities::lifecycle::api::RewindTarget::Delete(4),
    )
    .unwrap();
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&rebuilt), key(&h), "重建上下文必须与实时历史逐条一致");
}

#[test]
pub(crate) fn malformed_envelopes_are_classified_so_the_model_gets_the_right_fix() {
    // 类别是可判定的确切事实：模型据此能直接改对，而不是被笼统告知"JSON 不合法"。
    use crate::capabilities::llm::api::{Malformed, parse};
    // ① 字符串里直接换行（真实事故：write 的 content 里裸换行 → 整段 JSON 非法）
    let r = parse(
        "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"第一行\n第二行\"}}",
    );
    match r
        .tools
        .first()
        .cloned()
        .expect("应给出非法信封信号")
        .malformed
        .expect("应判定类别")
    {
        Malformed::RawControl { ch, line, tail } => {
            assert_eq!(ch, '\n', "要报出是哪个控制字符");
            assert_eq!(line, 1, "要报出在哪一行");
            assert!(tail.is_none(), "这一例括号是平衡的：{:?}", tail);
        }
        other => panic!("应判为裸控制字符：{:?}", other),
    }
    // ② 收尾未闭合（输出被截断）：要说清还差哪个字符，而不是笼统说"不完整"
    let r = parse("好。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\"}");
    assert_eq!(
        r.tools.first().cloned().expect("信号").malformed,
        Some(Malformed::Unclosed(crate::capabilities::llm::api::Tail {
            missing: "}".to_string(),
            in_string: false,
            envelopes: 1,
        }))
    );
    // 断在字符串中间：状态要说清"内容没写完"（补引号会拿到半截内容）
    let r = parse(
        "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"写了一半",
    );
    match r
        .tools
        .first()
        .cloned()
        .expect("信号")
        .malformed
        .expect("类别")
    {
        Malformed::Unclosed(t) => assert!(t.in_string, "要报出断在字符串里：{:?}", t),
        other => panic!("应判为未闭合：{:?}", other),
    }
    // ③ JSON 合法但字段不合法（缺 name）
    let r = parse("{\"type\":\"tool\",\"args\":{}}");
    match r
        .tools
        .first()
        .cloned()
        .expect("信号")
        .malformed
        .expect("类别")
    {
        Malformed::Shape(why) => assert!(why.contains("name"), "要说清缺哪个字段：{}", why),
        other => panic!("应判为字段不合法：{:?}", other),
    }
    // ④ 括号平衡但 JSON 语法非法：要带上解析器报的位置
    let r = parse("{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":,\"content\":\"x\"}}");
    match r
        .tools
        .first()
        .cloned()
        .expect("信号")
        .malformed
        .expect("类别")
    {
        Malformed::Syntax(why) => assert!(
            why.contains("line") && why.contains("column"),
            "要带位置：{}",
            why
        ),
        other => panic!("应判为语法错：{:?}", other),
    }
    // 四类各有各的修法（不是同一条笼统提示）
    let control = crate::capabilities::llm::api::malformed_report(&Malformed::RawControl {
        ch: '\n',
        line: 3,
        tail: None,
    });
    assert!(
        control.contains("裸换行") && control.contains("第 3 行"),
        "{}",
        control
    );
    assert!(
        control.contains("\\n"),
        "要教模型把换行写成反斜杠 n：{}",
        control
    );
    let shape = crate::capabilities::llm::api::malformed_report(&Malformed::Shape(
        "missing field name".to_string(),
    ));
    assert!(
        shape.contains("字段不合法") && shape.contains("missing field"),
        "{}",
        shape
    );
    assert!(
        shape.contains("calls"),
        "字段不合法也要示范 calls 数组形态：{}",
        shape
    );
    let tab = crate::capabilities::llm::api::malformed_report(&Malformed::RawControl {
        ch: '\t',
        line: 1,
        tail: None,
    });
    // 只差收尾括号：要说清"还差 }"（而不是误导成"内容过长"）
    let brace = crate::capabilities::llm::api::malformed_report(&Malformed::Unclosed(
        crate::capabilities::llm::api::Tail {
            missing: "}".to_string(),
            in_string: false,
            envelopes: 1,
        },
    ));
    assert!(brace.contains("还差 }"), "{}", brace);
    assert!(
        brace.contains("单数形态") && brace.contains("calls"),
        "未闭合也要示范单数与 calls 两种形态：{}",
        brace
    );
    assert!(
        !brace.contains("分次写入"),
        "内容写完就别引导它去分次写：{}",
        brace
    );
    // 断在字符串中间才是"内容没写完"，这时才谈分次写
    let cut = crate::capabilities::llm::api::malformed_report(&Malformed::Unclosed(
        crate::capabilities::llm::api::Tail {
            missing: "}\"}".to_string(),
            in_string: true,
            envelopes: 1,
        },
    ));
    // 一段回复里起了两段信封：要说清"只发一段"，而不是让它去补末尾括号（真实事故的形状）
    let multi = crate::capabilities::llm::api::malformed_report(&Malformed::Unclosed(
        crate::capabilities::llm::api::Tail {
            missing: "}}".to_string(),
            in_string: false,
            envelopes: 2,
        },
    ));
    assert!(
        multi.contains("2 段") && multi.contains("只发一段"),
        "{}",
        multi
    );
    assert!(
        !multi.contains("补上就完整了"),
        "两段时不能说「补上就完整」：{}",
        multi
    );
    assert!(
        cut.contains("断在字符串中间") && cut.contains("分次写入"),
        "{}",
        cut
    );
    assert!(tab.contains("制表符"), "{}", tab);
    assert_ne!(control, shape, "每一类的文案都要各说各的");
    assert_ne!(shape, brace, "每一类的文案都要各说各的");
    assert_ne!(brace, cut, "每一类的文案都要各说各的");
}

#[test]
pub(crate) fn envelope_tool_parses_name_and_args() {
    let r = crate::capabilities::llm::api::parse(TOOL_CALL);
    assert_eq!(r.verb, crate::capabilities::llm::api::Verb::Tool);
    let inv = r.tools.first().cloned().expect("应有调用申请");
    assert_eq!(inv.name, "scan");
    assert!(
        inv.module.is_none(),
        "没写 module = None（单模块 agent 靠这个兜底）"
    );
    assert!(inv.args_json.contains("keyword"));
    // 带 module 的信封：trim 后非空才是 Some（空串按省略处理）。
    let with_mod = crate::capabilities::llm::api::parse(
        "{\"type\":\"tool\",\"module\":\" reviewer \",\"name\":\"read_txt\",\"args\":{}}",
    );
    assert_eq!(
        with_mod
            .tools
            .first()
            .cloned()
            .expect("应有调用申请")
            .module
            .as_deref(),
        Some("reviewer")
    );
    let blank_mod = crate::capabilities::llm::api::parse(
        "{\"type\":\"tool\",\"module\":\"  \",\"name\":\"read_txt\",\"args\":{}}",
    );
    assert!(
        blank_mod
            .tools
            .first()
            .cloned()
            .expect("应有调用申请")
            .module
            .is_none()
    );
    // name 缺失 = 工具信封但不合法 → **独立的 malformed 信号**（不再按原文发言收录）。
    let bad = crate::capabilities::llm::api::parse("{\"type\":\"tool\",\"args\":{}}");
    assert_eq!(
        bad.verb,
        crate::capabilities::llm::api::Verb::Tool,
        "看得出是想发工具信封"
    );
    assert!(
        !bad.degraded,
        "malformed 与 degraded 是两回事（后者是信封缺失）"
    );
    let inv = bad.tools.first().cloned().expect("应给出非法信封信号");
    assert!(
        inv.malformed.is_some() && inv.name.is_empty(),
        "打捞不到名字就留空：{:?}",
        inv
    );
    assert!(bad.text.is_empty(), "非法信封的 JSON 也不进 text");
    // 真的"没有信封"仍然是 degraded say（原文收录）。
    let plain = crate::capabilities::llm::api::parse("没有信封的发言");
    assert!(plain.degraded && plain.tools.is_empty() && plain.text == "没有信封的发言");
    // 信封之外的正文才进 text（永不把信封 JSON 当文本）；只剩信封时 text 为空串。
    assert!(
        crate::capabilities::llm::api::parse(TOOL_CALL)
            .text
            .is_empty(),
        "只剩信封 → text 空"
    );
    let prose = crate::capabilities::llm::api::parse(
        "先看一眼。{\"type\":\"tool\",\"name\":\"scan\",\"args\":{}}后记",
    );
    assert_eq!(prose.text, "先看一眼。后记", "信封之外的正文进 text");
    assert!(
        !prose.text.contains('{') && !prose.text.contains("type"),
        "正文里不得残留 JSON：{}",
        prose.text
    );
}

#[test]
pub(crate) fn streaming_stops_at_the_envelope_brace() {
    // 正文开头照常流；一旦出现 "{"（信封开始）就不再外送后续片段。
    use crate::capabilities::conversation::session::api::stream_piece;
    let (send, acc) = stream_piece("", "我先看看。");
    assert_eq!(send, "我先看看。");
    let (send, acc) = stream_piece(&acc, "{\"type\":\"tool\"}");
    assert!(send.is_empty(), "信封不外泄");
    let (send, _) = stream_piece(&acc, "后记");
    assert!(send.is_empty(), "出现过花括号之后一律不外送");
    // { 出现在片段中间：它之前的部分仍可外送
    let (send, acc) = stream_piece("", "正文{后面是信封}");
    assert_eq!(send, "正文");
    let (send, _) = stream_piece(&acc, "尾巴");
    assert!(send.is_empty());
    // 没有花括号的正文一路外送
    let (send, acc) = stream_piece("你好", "，世界");
    assert_eq!(send, "，世界");
    assert_eq!(acc, "你好，世界");
}

#[test]
pub(crate) fn suppressed_stream_pieces_do_not_emit_delta_events() {
    use crate::capabilities::conversation::session::api::delta_visible;
    assert!(delta_visible("start", ""), "start 用来重置本轮流，必须外送");
    assert!(delta_visible("text", "正文"));
    assert!(delta_visible("reasoning", "想一想"));
    assert!(
        !delta_visible("text", ""),
        "信封被抑制的片段不该刷事件流（每秒几十条空 delta 会让界面假死）"
    );
    assert!(!delta_visible("reasoning", ""));
}

#[test]
pub(crate) fn reasoning_is_kept_on_final_transcript_lines() {
    let mut core = core_with_io_gateway(
        vec![module_of("a")],
        ReasoningGateway,
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "保留思维链",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let lines: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Transcript(ls) => Some(ls),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        lines
            .iter()
            .any(|l| l.reasoning.as_deref() == Some("先思考")),
        "定稿行应保留思维链：{:?}",
        lines
    );
}

#[test]
pub(crate) fn envelope_only_round_produces_only_a_tool_line() {
    // 只有信封、没有正文也没有思维链 → 只出 tool 行，不产生空行。
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
        .insert("scan".to_string(), decl("python tools/scan.py"));
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), runner);
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let rows = transcript_rows(
        &with_turn_io(|io, cancel, emit| {
            core.turn.single_say(
                &sid,
                "跑一下",
                io,
                cancel,
                emit,
                core.registry.as_ref(),
                core.probe.as_ref(),
            )
        })
        .unwrap(),
    );
    assert_eq!(rows.len(), 3, "用户行 / tool 行 / 答复行：{:?}", rows);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        1,
        "只出一条 tool 行：{:?}",
        rows
    );
    assert!(rows[1].2, "tool 行居中：{:?}", rows);
}

#[test]
pub(crate) fn an_aborted_generation_never_executes_a_repairable_envelope() {
    // 被停止的生成留下的"只差一个括号"的信封：即便修复端口能修，也绝不执行——
    // 半截信封是停下来的产物，不是模型的意图（真实会话里第一轮三次都是这个形状）。
    let note = s(&["w", "a", "note.txt"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"正文\"}}",
        note
    );
    let io = Arc::new(InMemorySysIo::new());
    // 中止网关：这一轮的通道回调返回 false（调用方要求停止）
    let mut core =
        core_with_io_gateway(vec![module_of("a")], AbortGateway { raw }, Arc::clone(&io));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    // 取消标志在生成开始前就已置位 = 用户按下了「停止」：引擎据此收尾，
    // 不再发起下一次调用（工具调用没有上限兜底，这条出口必须自己站住）。
    let events = {
        let io = crate::capabilities::conversation::session::api::TurnIo {
            llm: Default::default(),
            ask: None,
        };
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut noop = |_e: crate::capabilities::conversation::session::api::SessionEvent| {};
        core.turn
            .single_say(
                &sid,
                "写",
                &io,
                &cancel,
                &mut noop,
                core.registry.as_ref(),
                core.probe.as_ref(),
            )
            .unwrap()
    };
    let views = tool_views(&events);
    assert!(!views[0].ok, "被停止的生成不执行工具：{}", views[0].output);
    assert!(
        views[0].output.contains("还差"),
        "要如实说清信封没写完：{}",
        views[0].output
    );
    assert_eq!(io.get(&["w", "a", "note.txt"]), None, "绝不落盘");
}

#[test]
pub(crate) fn a_cancelled_generation_never_executes_a_valid_tool_envelope() {
    // 与上一条互补：信封本身**合法**，只是用户在生成途中按了停止。工具仍不得执行。
    let note = s(&["w", "a", "note.txt"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"正文\"}}}}",
        note
    );
    let io = Arc::new(InMemorySysIo::new());
    let mut core =
        core_with_io_gateway(vec![module_of("a")], AbortGateway { raw }, Arc::clone(&io));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = {
        let io = crate::capabilities::conversation::session::api::TurnIo {
            llm: Default::default(),
            ask: None,
        };
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut noop = |_e: crate::capabilities::conversation::session::api::SessionEvent| {};
        core.turn
            .single_say(
                &sid,
                "写",
                &io,
                &cancel,
                &mut noop,
                core.registry.as_ref(),
                core.probe.as_ref(),
            )
            .unwrap()
    };
    assert_eq!(
        io.get(&["w", "a", "note.txt"]),
        None,
        "取消后合法信封也绝不落盘"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            crate::capabilities::conversation::session::api::SessionEvent::Transcript(lines)
                if lines.iter().any(|l| l.kind == "notice" && l.line.contains("[停止]"))
        )),
        "取消要留下可回档的 [停止] 行：{:?}",
        events
    );
}

/// 目的：用户按「中断」时已经流到界面上的那半句**不能凭空消失**：它留成一条转录行、标「（已中断）」，
///   但**不进对话**——点「继续」是从断点重开一轮，不是把这半句当发言续写。
#[test]
pub(crate) fn an_interrupted_generation_keeps_the_half_sentence_as_a_cut_line() {
    use crate::capabilities::conversation::session::api::SessionEvent;
    let io = Arc::new(InMemorySysIo::new());
    let mut core = core_with_io_gateway(
        vec![module_of("a")],
        AbortGateway {
            raw: "我先写第一段".to_string(),
        },
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = {
        let io = crate::capabilities::conversation::session::api::TurnIo {
            llm: Default::default(),
            ask: None,
        };
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut noop = |_e: SessionEvent| {};
        core.turn
            .single_say(
                &sid,
                "写",
                &io,
                &cancel,
                &mut noop,
                core.registry.as_ref(),
                core.probe.as_ref(),
            )
            .unwrap()
    };
    let cut = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::Transcript(lines) => lines.iter().find(|l| l.cut).cloned(),
            _ => None,
        })
        .expect("被中断的半句要留档：没有 cut 行");
    assert!(
        cut.line.contains("我先写第一段"),
        "半句原样留下：{}",
        cut.line
    );
    assert!(
        cut.line.contains("（已中断）"),
        "要标出它没写完：{}",
        cut.line
    );
    assert_eq!(
        cut.kind, "msg",
        "它还是模型发言那一行，不伪装成提示：{}",
        cut.kind
    );
    let history = core.single_history(&sid).expect("会话还在");
    assert!(
        history.iter().all(|m| !m.content.contains("我先写第一段")),
        "半句不进对话（否则「继续」会把半句当发言接着写）：{:?}",
        history
    );
}

#[test]
pub(crate) fn a_truncated_output_is_reported_as_truncation_not_as_a_bad_envelope() {
    // 供应商说 finish_reason=length：回执要指出"是被按长度截断"，而不是让模型去查括号；
    // 真实会话里正是分辨不出这两者，模型照着"内容过长"的假设白跑了两轮。
    let broken = "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":";
    let mut core = core_with_gateway(
        vec![module_of("a")],
        TruncGateway {
            script: vec![
                broken.to_string(),
                "{\"type\":\"say\",\"text\":\"知道了\"}".to_string(),
            ],
        },
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "写",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let views = tool_views(&events);
    assert!(!views[0].ok);
    assert!(
        views[0].output.contains("被供应商按输出长度截断"),
        "{}",
        views[0].output
    );
    assert!(
        views[0].output.contains("还差"),
        "还差什么也要说：{}",
        views[0].output
    );

    // 纯文本轮被截断：行尾如实标注（与"已停止"同一套做法），模型与用户都看得到
    let mut core2 = core_with_gateway(
        vec![module_of("a")],
        TruncGateway {
            script: vec!["半句话".to_string()],
        },
    );
    let sid2 = core2.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events2 = with_turn_io(|io, cancel, emit| {
        core2.turn.single_say(
            &sid2,
            "说",
            io,
            cancel,
            emit,
            core2.registry.as_ref(),
            core2.probe.as_ref(),
        )
    })
    .unwrap();
    let rows = transcript_rows(&events2);
    assert!(
        rows.iter()
            .any(|r| r.1.contains("半句话") && r.1.contains("（本段被输出长度截断）")),
        "{:?}",
        rows
    );
}

#[test]
pub(crate) fn a_freeform_envelope_is_never_repaired() {
    // 自由格式工具的信封本身不合法时：不做信封修复（修会把正文里的换行当作字符串内容转义掉），
    // 如实报"信封没写完"，也绝不落盘。
    let target = s(&["w", "m0", "out.md"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"name\":\"patch\"\n*** Add File: {}\n正文第一行\n正文第二行\n*** End File\n",
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
    let sid = core.create_work(work("w", SINGLE, &["m0"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "打补丁",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let views = tool_views(&events);
    assert!(!views[0].ok, "{}", views[0].output);
    assert!(
        views[0].output.contains("还差"),
        "要报信封本身没写完：{}",
        views[0].output
    );
    assert!(
        !views[0].output.contains("信封修复"),
        "自由格式不做信封修复：{}",
        views[0].output
    );
    assert_eq!(io.get(&["w", "m0", "out.md"]), None, "绝不落盘");
}

/// 目的：手写信封通道的**批量调用**：一封 calls 数组里的多个调用各成一条工具行，结果按原序回填，
///   且与原生通道一样「重建出来必须与实时逐条一致」（手写信封不涉及 role=tool）。
#[test]
pub(crate) fn envelope_multi_call_runs_every_call_and_rebuilds_identically() {
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "a.txt"], "A1\n");
    io.seed(&["w", "a", "b.txt"], "B1\n");
    let a = s(&["w", "a", "a.txt"]);
    let b = s(&["w", "a", "b.txt"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"calls\":[{{\"name\":\"read\",\"args\":{{\"path\":\"{}\"}}}},{{\"name\":\"read\",\"args\":{{\"path\":\"{}\"}}}}]}}",
        a, b
    );
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            raw.clone(),
            "{\"type\":\"say\",\"text\":\"读完了\"}".to_string(),
        ],
    );
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "读两个文件",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let views = tool_views(&events);
    assert_eq!(
        views.len(),
        2,
        "两个调用两条工具行：{:?}",
        views.iter().map(|v| v.name.clone()).collect::<Vec<_>>()
    );
    assert!(
        views[0].output.contains("A1") && views[1].output.contains("B1"),
        "结果按原序回填"
    );
    assert!(views[0].call_id.is_empty(), "手写信封没有原生调用 id");
    assert_eq!(views[0].reply, views[1].reply, "同一次回复的工具行同号");
    assert_eq!(
        transcript_raws(&events),
        vec![raw.clone()],
        "原文是回复级事实：两个调用也只落一份"
    );
    let live = core.single_history(&sid).unwrap();
    assert!(
        live.iter().all(|m| m.role != "tool"),
        "手写信封通道不发 role=tool"
    );
    assert_eq!(
        live.iter()
            .filter(|m| m.role == "user" && m.content.contains("[工具结果]"))
            .count(),
        2,
        "两条结果各发一条用户消息：{:?}",
        live
    );

    // 「重启」：同一份落盘历史交给新核心，重建上下文必须与实时逐条一致。
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    crate::tests::lifecycle::rewind(
        &mut core2,
        &sid,
        crate::capabilities::lifecycle::api::RewindTarget::Delete(
            transcript_rows(&events).len() as u64
        ),
    )
    .unwrap();
    let rebuilt = core2.single_history(&sid).unwrap();
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| (m.role.clone(), m.content.clone(), m.tool_calls.len()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live),
        "重建上下文必须与实时历史逐条一致"
    );
}

/// 目的：两种信封形态**互斥**：一封里既有 name 又有 calls = 字段不合法 → 记一条失败工具行、一个工具都不执行。
#[test]
pub(crate) fn envelope_rejects_mixing_the_single_and_calls_shapes() {
    let io = Arc::new(InMemorySysIo::new());
    let out = s(&["w", "a", "out.md"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"x\"}},\"calls\":[{{\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"y\"}}}}]}}",
        out, out
    );
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            raw.clone(),
            "{\"type\":\"say\",\"text\":\"知道了\"}".to_string(),
        ],
    );
    let mut core = core_with_io_gateway(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "写",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let views = tool_views(&events);
    assert_eq!(views.len(), 1, "只记一条失败工具行");
    assert!(!views[0].ok, "混用两种形态必须失败");
    assert!(views[0].output.contains("互斥"), "{}", views[0].output);
    assert_eq!(
        io.get(&["w", "a", "out.md"]),
        None,
        "一个工具都不执行（绝不落盘）"
    );
}
