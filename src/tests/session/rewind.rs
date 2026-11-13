//! 目的：回档、归档与转录重写。
//! 管：按回复原子的回档、tail 折叠与恢复、删除后重写、重建上下文与 shell 快照。
//! 不管：会话创建；压缩策略（见 session/domain/session.rs）。
//! 联动：`tests::builders`；转录格式见 docs/session/session-model.md。

use super::super::builders::*;
use super::super::prelude::*;

#[test]
pub(crate) fn direct_rewind_drops_tail_then_continue_allows_user_turn() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"答一\"}".to_string(),
            "{\"type\":\"say\",\"text\":\"答二\"}".to_string(),
            "{\"type\":\"say\",\"text\":\"续答\"}".to_string(),
        ],
    );
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap(); // 行 0=用户, 1=AI
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap(); // 行 2=用户, 3=AI

    // 末条是 AI → 继续被拦，只给提醒、不发请求
    let blocked = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(matches!(&blocked[0], SessionEvent::Notice(n) if n.contains("最后一条是 AI 发言")));

    // 回档到第 1 行：保留前 1 行（= 删第 1 行及其后），只留「用户·问一」
    let replayed = core
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(1),
        )
        .unwrap();
    let lines = replay_lines(&replayed);
    assert_eq!(lines, vec!["[用户] 问一".to_string()]);

    // 末条是用户 → 继续直接续跑（不再需要用户发言）
    let ev = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    let reply = ev
        .iter()
        .find_map(|e| match e {
            SessionEvent::Transcript(l) => l.first().map(|x| x.render()),
            _ => None,
        })
        .expect("应有转录回复");
    assert!(
        reply.contains("续答"),
        "继续应直接用现有历史问模型：{}",
        reply
    );

    // 落盘历史也按回档截断回放
    let (_, events) = core.history_open("w").unwrap();
    assert_eq!(
        replay_lines(&events),
        vec!["[用户] 问一".to_string(), format!("[a] {}", "续答")]
    );
}

/// 目的：留档：只标记并折叠尾部，文件字节一个不少，新行从最大 id 续号；恢复把标记及其后真的删掉。
#[test]
pub(crate) fn archive_folds_the_tail_and_restore_brings_it_back() {
    use crate::capabilities::conductor::api::RewindTarget;
    use crate::capabilities::session::api::next_line_id;
    let say = |t: &str| serde_json::json!({"type": "say", "text": t}).to_string();
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![say("答一"), say("答二")]);
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap(); // 行 0 用户 / 1 AI
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap(); // 行 2 用户 / 3 AI

    let rows = |raw: &[serde_json::Value]| -> Vec<u64> {
        raw.iter()
            .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
            .flatten()
            .map(|l| l.get("id").and_then(|i| i.as_u64()).unwrap_or(0))
            .collect()
    };

    // 留档到第 1 行：活动视图只剩「用户 问一」；文件里 2/3 仍在，标记写进流水。
    let replayed = core.rewind(&sid, RewindTarget::Archive(1)).unwrap();
    assert_eq!(replay_lines(&replayed), vec!["[用户] 问一".to_string()]);
    let (_, raw) = hist.load("w").unwrap();
    assert_eq!(rows(&raw), vec![0, 1, 2, 3], "留档不删字节");
    assert_eq!(next_line_id(&raw), 4, "新行从最大 id 续号，绝不复用");
    let mark = raw
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("rewind"))
        .expect("留档标记要进流水");
    assert_eq!(
        mark.get("shared").and_then(|v| v.as_u64()),
        None,
        "没有提交时共享区头是空的"
    );
    assert_eq!(
        mark.get("heads").and_then(|h| h.get("a")),
        Some(&serde_json::Value::Null),
        "每个 agent 的头要写进标记：{:?}",
        mark
    );
    assert_eq!(
        mark.get("label").and_then(|v| v.as_str()),
        Some("答二"),
        "标签取折叠区间里最后一条内容行"
    );
    assert_eq!(
        crate::capabilities::session::api::truncate_events(&raw)
            .iter()
            .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
            .flatten()
            .count(),
        1,
        "活动视图只剩折叠外的行"
    );

    // 恢复标记 1：活动视图回到全部 4 行；标记与它之后的内容**真的**从文件里消失。
    let back = core.rewind(&sid, RewindTarget::Restore(1)).unwrap();
    assert_eq!(
        replay_lines(&back),
        vec![
            "[用户] 问一".to_string(),
            "[a] 答一".to_string(),
            "[用户] 问二".to_string(),
            "[a] 答二".to_string()
        ]
    );
    let (_, raw2) = hist.load("w").unwrap();
    assert_eq!(rows(&raw2), vec![0, 1, 2, 3]);
    assert!(
        !raw2
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("rewind")),
        "恢复真的删掉了标记"
    );
}

/// 目的：删除：真的把标记点之后的字节从文件里删掉（不是只截视图）。
#[test]
pub(crate) fn delete_rewrites_the_file_for_real() {
    use crate::capabilities::conductor::api::RewindTarget;
    let say = |t: &str| serde_json::json!({"type": "say", "text": t}).to_string();
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![say("答一"), say("答二")]);
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap();
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap();

    core.rewind(&sid, RewindTarget::Delete(2)).unwrap();
    let (_, raw) = hist.load("w").unwrap();
    let ids: Vec<u64> = raw
        .iter()
        .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
        .flatten()
        .map(|l| l.get("id").and_then(|i| i.as_u64()).unwrap_or(0))
        .collect();
    assert_eq!(ids, vec![0, 1], "删除真的截断文件");
    assert!(
        !raw.iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("rewind")),
        "删除不留标记"
    );
}

#[test]
pub(crate) fn rewind_keeps_only_lines_before_the_mark() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"答一\"}".to_string(),
            "{\"type\":\"say\",\"text\":\"答二\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap(); // 行 0 用户 / 1 AI
    with_live(|l| core.single_say(&sid, "问二", l)).unwrap(); // 行 2 用户 / 3 AI

    // 回档到第 2 行 = 删第 2 行及其后 → 只留前 2 行，历史与 marks 同步截断
    let replayed = core
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(2),
        )
        .unwrap();
    assert_eq!(
        replay_lines(&replayed),
        vec!["[用户] 问一".to_string(), "[a] 答一".to_string()]
    );
    assert_eq!(
        core.single_history(&sid).unwrap().len(),
        2,
        "用户 + 答一（身份由参数现渲染，不占对话）"
    );

    // 回档到第 0 行 = 转录清空、对话也清空（身份由参数现渲染，不在对话里）
    let replayed = core
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(0),
        )
        .unwrap();
    assert!(
        replay_lines(&replayed).is_empty(),
        "点第一行 → 转录清空：{:?}",
        replayed
    );
    assert!(core.single_history(&sid).unwrap().is_empty(), "对话清空");
    assert!(core.single_identity(&sid).is_some(), "身份块不受回档影响");

    // next_line 归零：下一条行 id 从 0 开始
    let ev = with_live(|l| core.single_say(&sid, "再来", l)).unwrap();
    let first = transcript_rows(&ev).first().cloned().expect("应有新行");
    assert_eq!(first.0, 0, "回档清空后行 id 从头计");
}

/// 目的：钉住连续删除回档时 line_reply / 工具行锚与 marks 一起回退（第二次删除到 0 不 panic）。
#[test]
pub(crate) fn repeated_delete_to_zero_keeps_derived_state_in_sync() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"答一\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "问一", l)).unwrap(); // 行 0 用户 / 1 AI

    core.rewind(
        &sid,
        crate::capabilities::conductor::api::RewindTarget::Delete(0),
    )
    .unwrap();
    assert!(core.single_history(&sid).unwrap().is_empty(), "对话清空");

    // marks 已清空、line_reply 仍保留 2 项时，第二次删除到 0 会走 align_keep 的 MAX 分支；
    // 这里钉住它不 panic。
    let again = core
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(0),
        )
        .expect("重复删除到 0 不应 panic");
    assert!(again.is_empty(), "清空后重复回档仍是空视图");
    assert!(core.single_history(&sid).unwrap().is_empty(), "仍是空对话");

    // 回档后还能继续；行号从 0 重新计，不会被陈旧的 line_reply 拖到越界。
    let ev = with_live(|l| core.single_say(&sid, "再来", l)).unwrap();
    let rows = transcript_rows(&ev);
    assert_eq!(
        rows.first().map(|r| r.0),
        Some(0),
        "回档清空后行 id 从 0 起：{:?}",
        rows
    );
    assert_eq!(
        core.single_history(&sid).unwrap().len(),
        2,
        "用户 + 答一（身份不占对话）"
    );
}

#[test]
pub(crate) fn rebuilt_context_keeps_tool_result() {
    // 同一份落盘历史 + 新的 Conductor 模拟「重启」：重建上下文时工具子轮不能丢。
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let note = s(&["w", "a", "note.txt"]);
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        format!("{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"你好\"}}}}", note),
        "{\"type\":\"say\",\"text\":\"写完了\"}".to_string(),
    ]);
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    drop(core); // 会话随进程消失，只剩落盘流水

    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    // 内存里没有这个会话 → 走 rebuild_session（保留全部 3 行）
    core2
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(3),
        )
        .unwrap();
    let h = core2.single_history(&sid).expect("重建后应在内存里");
    assert!(
        h.iter()
            .any(|m| m.role == "assistant" && m.content.contains("\"type\":\"tool\"")),
        "重建要还原模型原始工具请求：{:?}",
        h
    );
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "重建必须保留工具结果（这条正是要修的 bug）：{:?}",
        h
    );
    assert_eq!(
        io.get(&["w", "a", "note.txt"]).as_deref(),
        Some("你好"),
        "重建后沙箱里的成品仍在"
    );
    assert!(
        h.iter()
            .any(|m| m.role == "assistant" && m.content.contains("写完了"))
    );
}

/// 目的：重建会话时 shell 状态栏快照按转录回填（重启 / 启用后不再"没有 chip"）。
#[test]
pub(crate) fn rebuild_restores_the_last_shell_snapshot_from_the_transcript() {
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    hist.append(
        &sid,
        &[serde_json::json!({
            "type": "transcript",
            "lines": [{
                "id": 0,
                "tool": {
                    "speaker": "a", "module": "", "name": "shell", "ok": true,
                    "args": "{\"command\":\"npm test\",\"scope\":\"sandbox\"}",
                    "output": "ok\n[shell] 退出码 0\n[shell] cwd /w/a · 耗时 7 秒"
                }
            }]
        })],
    )
    .unwrap();
    let (meta, events) = core.history_open(&sid).unwrap();
    drop(core.rebuild_session(&meta, &events).expect("按转录重建"));
    let live = core.shell_view(&sid).expect("重建后状态栏要有 shell 快照");
    assert_eq!(live.command, "npm test");
    assert_eq!(live.cwd, "/w/a");
    assert_eq!(live.elapsed_secs, Some(7));
    assert_eq!(live.exit_code, Some(0));
    assert!(live.historical, "派生快照要标历史");
}

/// 目的：新转录里 shell 带着结构化事实（回执被截断、没有尾行）时，重建读事实而不是解析文本。
#[test]
pub(crate) fn rebuild_prefers_the_structured_shell_facts_over_the_receipt() {
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    hist.append(
        &sid,
        &[serde_json::json!({
            "type": "transcript",
            "lines": [{
                "id": 0,
                "tool": {
                    "speaker": "a", "module": "", "name": "shell", "ok": true,
                    "args": "{\"command\":\"npm test\",\"scope\":\"sandbox\"}",
                    "output": "ok（回执被截断，没有尾行）",
                    "shell": {
                        "cwd": "/w/b", "elapsed_ms": 331_400,
                        "fence_prep_ms": 327_000, "exit_code": 0,
                        "status": "exit", "truncated": true
                    }
                }
            }]
        })],
    )
    .unwrap();
    let (meta, events) = core.history_open(&sid).unwrap();
    drop(core.rebuild_session(&meta, &events).expect("按转录重建"));
    let live = core.shell_view(&sid).expect("重建后状态栏要有 shell 快照");
    assert_eq!(live.command, "npm test");
    assert_eq!(live.cwd, "/w/b", "读事实，不回落到解析");
    assert_eq!(live.elapsed_secs, Some(331));
    assert_eq!(live.fence_prep_secs, Some(327));
    assert_eq!(live.exit_code, Some(0));
    assert_eq!(live.status.as_deref(), Some("exit"));
    assert!(live.historical);
}

/// 目的：回档**按回复原子**：截在一次回复中间时整条回复一起丢，绝不留下"孤儿工具结果"。
#[test]
pub(crate) fn rewind_never_splits_a_reply() {
    use crate::capabilities::llm::api::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "a.txt"], "A1\n");
    io.seed(&["w", "a", "b.txt"], "B1\n");
    let a = s(&["w", "a", "a.txt"]);
    let b = s(&["w", "a", "b.txt"]);
    let steps = vec![
        NativeStep::Calls(vec![
            ToolCall {
                id: "c1".to_string(),
                name: "read".to_string(),
                args_json: format!("{{\"path\":\"{}\"}}", a),
            },
            ToolCall {
                id: "c2".to_string(),
                name: "read".to_string(),
                args_json: format!("{{\"path\":\"{}\"}}", b),
            },
        ]),
        NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
    ];
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![steps]),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.registry_mut().probe_model_tools("m").expect("探测");
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core.single_say(&sid, "读两个文件", l)).unwrap();

    // 行序：0 用户 / 1 工具 / 2 工具 / 3 答复。回档到第 2 行 = 落在回复内部 → 整条回复一起丢。
    let replayed = core
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(2),
        )
        .unwrap();
    assert_eq!(
        replay_lines(&replayed),
        vec!["[用户] 读两个文件".to_string()],
        "截在回复中间要把该回复的工具行与答复行一起丢掉：{:?}",
        replay_lines(&replayed)
    );
    let history = core.single_history(&sid).unwrap();
    assert_eq!(
        history.len(),
        1,
        "对话 = 用户（身份不占对话）：{:?}",
        history
    );
    assert!(
        history.iter().all(|m| m.tool_call_id.is_empty()),
        "历史里不许出现没有对应助手消息的孤儿工具结果：{:?}",
        history
    );
}

/// 目的：侧栏顺序 = **树序**：父会话紧跟它的子会话（顺序与缩进同源，不会再错位）。
#[test]
pub(crate) fn history_list_is_ordered_as_a_tree() {
    let hv = |name: &str, parent: Option<&str>, ts: i64| {
        crate::capabilities::session::api::HistoryView {
            name: name.to_string(),
            mode: "collab".to_string(),
            ts,
            done: false,
            exec: Default::default(),
            parent: parent.map(|s| s.to_string()),
            workspace: None,
            workspace_missing: false,
            run: RunState::Active,
        }
    };
    // 顶层 A(10) 比 B(5) 新；A 下两个子会话（甲=9 比 乙=8 新）。
    let got = crate::capabilities::conductor::service::tree_order(vec![
        hv("A", None, 10),
        hv("B", None, 5),
        hv("A--甲", Some("A"), 9),
        hv("A--乙", Some("A"), 8),
    ]);
    let names: Vec<&str> = got.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["A", "A--甲", "A--乙", "B"],
        "父会话必须紧跟它的子会话"
    );
}
