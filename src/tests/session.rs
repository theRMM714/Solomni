//! 会话能力测试：建立与落盘、回合、回档、历史与文件视图、编辑与冻结。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

#[test]
pub(crate) fn create_work_persists_and_history_replays() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    let sid = core
        .create_work(work("工作一", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert_eq!(sid, "工作一");
    with_live(|l| core.single_say(&sid, "你好", l)).unwrap();
    let list = core.history_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "工作一");
    assert_eq!(list[0].mode, "single");
    let (meta, events) = core.history_open("工作一").unwrap();
    assert_eq!(meta.mode, "single");
    assert!(
        events
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript")),
        "历史里应有转录事件"
    );
    // 重名（磁盘已存在）被拒
    assert!(core
        .create_work(work("工作一", WorkMode::Single, &["a"]))
        .unwrap_err()
        .contains("已存在"));
    // 删除会话：内存与磁盘一起删，之后名字可复用
    assert!(core.history_delete("工作一").unwrap());
    assert!(core.history_list().is_empty());
    assert!(core
        .create_work(work("工作一", WorkMode::Single, &["a"]))
        .is_ok());
}

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
    let replayed = core.rewind(&sid, 1).unwrap();
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
    let replayed = core.rewind(&sid, 2).unwrap();
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
    let replayed = core.rewind(&sid, 0).unwrap();
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
    core2.rewind(&sid, 3).unwrap();
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
    assert!(h
        .iter()
        .any(|m| m.role == "assistant" && m.content.contains("写完了")));
}

#[test]
pub(crate) fn create_work_validates_user_choices() {
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    assert!(core
        .create_work(work("", WorkMode::Single, &["a"]))
        .unwrap_err()
        .contains("工作名不能为空"));
    assert!(core
        .create_work(work("x", WorkMode::Single, &["ghost"]))
        .unwrap_err()
        .contains("无此模块"));
    assert!(core
        .create_work(work("x", WorkMode::Single, &[]))
        .unwrap_err()
        .contains("至少要有一个模块"));
    // 单模式的**组合语义**：点名多个 = 并成一个临时组合（模块去重、保序；模型取核心默认）。
    let two_agents = WorkSpec {
        name: "x".to_string(),
        mode: WorkMode::Single,
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
                modules: vec!["b".to_string()],
                model: None,
            },
        ],
        task: None,
        delegate: false,
    };
    let opened = core
        .create_work(two_agents)
        .expect("单模式点名多个应并成一个，不是报错");
    assert_eq!(opened.agents, vec!["x".to_string()], "并出来的组合用工作名");
    let meta = core.history_open("x").expect("读 meta").0;
    assert_eq!(
        meta.agents[0].modules,
        vec!["a".to_string(), "b".to_string()]
    );
    assert!(meta.agents[0].transient, "并出来的是临时 agent");
    let _ = core.history_delete("x");
    assert!(core
        .create_work(collab_work("x", &["a"], false, "  "))
        .unwrap_err()
        .contains("必须填写本次需求"));
    let mut bad_model = work("x", WorkMode::Single, &["a"]);
    bad_model.agents[0].model = Some("ghost".to_string());
    assert!(core
        .create_work(bad_model)
        .unwrap_err()
        .contains("无此模型"));
    // 建一次成功后重名被拒
    assert!(core
        .create_work(work("同名", WorkMode::Single, &["a"]))
        .is_ok());
    assert!(core
        .create_work(work("同名", WorkMode::Single, &["a"]))
        .unwrap_err()
        .contains("已存在"));
    // 非法字符（将来要当目录名）
    assert!(core
        .create_work(work("a/b", WorkMode::Single, &["a"]))
        .unwrap_err()
        .contains("不能包含"));
}

/// 设置是流式的**上限**：设置关掉时，调用方要流式也拿不到（全局通用）。
#[test]
pub(crate) fn streaming_setting_is_the_ceiling_for_every_call() {
    let core = crate::tests::doubles::core_with_settings(InMemorySettings::with_llm(false, 77));
    assert!(!core.llm_opts(true).stream, "设置关掉时一律非流式");
    assert_eq!(core.llm_opts(true).timeout_secs, 77, "预算来自设置");
    let core2 = crate::tests::doubles::core_with_settings(InMemorySettings::with_llm(true, 88));
    assert!(core2.llm_opts(true).stream, "设置打开且调用方要流式");
    assert!(!core2.llm_opts(false).stream, "调用方可以在本次放弃流式");
    assert_eq!(core2.llm_opts(false).timeout_secs, 88, "预算与流式开关无关");
}

/// 用户进 agent 会话说的话，**下一回合它带着**——"讨论与执行不分家"的核心承诺。
/// 判据在"回合收到的消息"上：主会话内容靠**注入**（讨论上下文）给各 agent，
/// 而用户在某个 agent 会话说的话进**它自己的历史**，下一回合一起带上。
#[test]
pub(crate) fn discussion_turn_carries_the_agent_sessions_own_history() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut chat = super::RecordingChat {
        inner: scripted(vec!["{\"type\":\"say\",\"text\":\"收到\"}".into()]),
        seen: Arc::clone(&seen),
    };
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prompts = test_prompts();
    let turn = crate::capabilities::collab::domain::engine::Discussion::turn_with(
        &*test_tools_svc(),
        "discussant",
        &cancel,
        crate::capabilities::llm::api::CompleteOpts::plain(false),
        "a",
        "（测试）身份",
        &[crate::capabilities::llm::api::Msg::user("只看第二份资料")],
        &mut chat,
        None,
        vec![crate::capabilities::llm::api::Msg::user("讨论上下文")],
        &prompts.tools(),
        &mut |_| {},
    )
    .expect("跑一个回合");
    assert!(matches!(
        turn.verb,
        Some(crate::capabilities::llm::api::Verb::Say)
    ));
    let got = seen.lock().expect("锁").clone();
    assert!(
        got.iter()
            .any(|msgs| msgs.iter().any(|c| c.contains("只看第二份资料"))),
        "下一回合该带着用户那句话：{got:?}"
    );
    assert!(
        got.iter()
            .any(|msgs| msgs.iter().any(|c| c.contains("讨论上下文"))),
        "讨论上下文照旧注入：{got:?}"
    );
}

#[test]
pub(crate) fn core_direct_seeds_system_prompt() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"你好\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    // 回归：单 agent 会话的身份块必须是职责提示词（由会话参数现渲染，不进对话列表）。
    let identity = core.single_identity(&sid).expect("身份块");
    assert!(identity.contains("你负责a"), "{}", identity);
    assert!(
        core.single_history(&sid).unwrap().is_empty(),
        "对话里只有真正发生过的事（此刻还没有）"
    );
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    // 回归：用户发言必须入转录（此前只进历史、不进转录，历史回放会丢用户消息）。
    match &events[0] {
        SessionEvent::Transcript(lines) => {
            assert_eq!(lines[0].kind, "user");
            assert_eq!(lines[0].speaker, "用户");
            assert_eq!(lines[0].line, "在吗");
        }
        _ => panic!("首条应为用户转录行"),
    }
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.speaker == "a"))));
}

#[test]
pub(crate) fn single_mode_accepts_multi_module_agent_and_converses() {
    // 单 agent 形态允许 1 个 agent 带多个模块：能建、system 并入全部职责、说话人是 agent 名。
    let mut member = BTreeMap::new();
    member.insert(
        "组合".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"收到\"}".to_string()],
    );
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(member, vec!["[]".into()]),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a", "b"]))
        .unwrap()
        .sid;
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.mode, "single");
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(
        meta.agents[0].modules,
        vec!["a".to_string(), "b".to_string()]
    );
    let identity = core.single_identity(&sid).expect("身份块");
    assert!(
        identity.contains("你负责a") && identity.contains("你负责b"),
        "身份块必须并入全部模块职责：{}",
        identity
    );
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    assert!(
        events.iter().any(
            |e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.speaker == "组合"))
        ),
        "转录说话人必须是 agent 名：{:?}",
        events
    );
}

/// **落盘策略由会话种类定、判定只有一处**：短暂事件任何会话都不留；系统会话（`#` 开头）一条都不留。
#[test]
pub(crate) fn persist_policy_is_decided_by_the_session_kind() {
    use crate::capabilities::conductor::service::{is_system_session, PersistPolicy};
    use crate::capabilities::session::api::SessionEvent;
    let line = SessionEvent::Transcript(vec![crate::capabilities::session::api::LineView::system(
        "",
        "x".to_string(),
    )]);
    let delta = SessionEvent::Delta {
        speaker: "a".to_string(),
        kind: "text".to_string(),
        text: "x".to_string(),
    };
    assert!(PersistPolicy::Keep.keeps(&line), "定稿行要留");
    assert!(!PersistPolicy::Keep.keeps(&delta), "短暂事件不留");
    assert!(!PersistPolicy::Drop.keeps(&line), "系统会话一条都不留");
    assert!(
        is_system_session("#suggest") && !is_system_session("w"),
        "# 开头 = 系统会话"
    );
}

#[test]
pub(crate) fn workspace_list_reports_work_and_agent_files() {
    let ws = InMemoryWorkspace::new();
    ws.seed("w", "work", "投喂.txt");
    ws.seed("w", "work", "sub/b.txt");
    ws.seed("w", "甲", "note.txt");
    ws.seed("w", "乙", "x.txt");
    let f = ws
        .list("w", &["甲".to_string(), "乙".to_string(), "丙".to_string()])
        .unwrap();
    assert_eq!(
        f.work,
        vec!["sub/b.txt".to_string(), "投喂.txt".to_string()],
        "work 清单排序稳定"
    );
    assert_eq!(f.agents["甲"], vec!["note.txt".to_string()]);
    assert_eq!(f.agents["乙"], vec!["x.txt".to_string()]);
    assert!(
        f.agents["丙"].is_empty(),
        "没有文件的 agent 也要在清单里（空表）"
    );
}

#[test]
pub(crate) fn core_files_view_carries_lists_and_real_roots() {
    let ws = Arc::new(InMemoryWorkspace::new());
    let mut core = core_with_workspace(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::clone(&ws),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    ws.seed("w", "work", "投喂.txt");
    ws.seed("w", "a", "note.txt");
    ws.seed("w", "b", "别人的.txt"); // 不属于本工作的 agent → 不该出现
    let f = core.files_view(&sid).unwrap();
    assert_eq!(f.work, vec!["投喂.txt".to_string()]);
    assert_eq!(f.agents.len(), 1, "只列本工作的 agent：{:?}", f.agents);
    assert_eq!(f.agents[0].name, "a");
    assert_eq!(f.agents[0].files, vec!["note.txt".to_string()]);
    // roots：真实绝对路径、/ 书写形式、与 agents 同序同名
    assert_eq!(
        f.roots.work,
        s(&["w", "work"]),
        "共享区根 = Sandboxes.shared"
    );
    assert_eq!(f.roots.agents.len(), f.agents.len());
    assert_eq!(
        f.roots
            .agents
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        f.agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        "roots.agents 与 agents 必须同序同名"
    );
    assert_eq!(
        f.roots.agents[0].root,
        s(&["w", "a"]),
        "agent 根 = 它的私有沙箱"
    );
    for root in std::iter::once(&f.roots.work).chain(f.roots.agents.iter().map(|r| &r.root)) {
        assert!(
            std::path::Path::new(root).is_absolute(),
            "根必须是绝对路径：{}",
            root
        );
        assert!(!root.contains('\\'), "根一律 / 分隔：{}", root);
        assert!(!root.starts_with("\\\\?\\"), "不得带扩展长度前缀：{}", root);
    }
    assert!(
        core.files_view("不存在的工作").is_err(),
        "无此会话要如实报错"
    );
}

#[test]
pub(crate) fn core_direct_tool_flow_injects_result_into_history() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "  3 | 依据行".into(),
        ok: true,
    });
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.to_string(),
            "{\"type\":\"say\",\"text\":\"依据第 3 行，结论成立\"}".to_string(),
        ],
    );
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), decl("python tools/grep.py"));
    let mut mod_a = module_of("a");
    mod_a.manifest.tools = manifest_tools;
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "核对一下", l)).unwrap();
    // 一行 = 一轮模型调用：用户行 / tool 行 / 最终文本行，id 连续且 tool 行夹在中间。
    let rows = transcript_rows(&events);
    assert_eq!(rows.len(), 3, "一次工具循环应得到 3 条行：{:?}", rows);
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "id 连续"
    );
    assert!(
        !rows[0].2 && rows[1].2 && !rows[2].2,
        "tool 行夹在中间：{:?}",
        rows
    );
    assert!(
        rows[1].1.contains("grep") && rows[1].1.contains("成功"),
        "{:?}",
        rows[1]
    );
    assert!(rows[2].1.contains("结论成立"), "{:?}", rows[2]);
    // 历史完整：用户消息 → 工具信封原文 → 工具结果 → 最终答复。
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] a.grep")),
        "工具结果必须回注上下文（标签带模块）：{:?}",
        h
    );
    assert!(h
        .iter()
        .any(|m| m.role == "assistant" && m.content.contains("结论成立")));
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}

// ---------- 内置文件工具：沙箱寻址与越界 ----------

/// **派发行：界面是系统行，上下文是 user 角色**——Web 与 CLI 走同一条语义。
/// 界面上它是核心说的话（不得显示成"用户"），但请求里必须有一条 user 消息：
/// 一条 user 都没有的请求会被供应商**整条拒收**（真机 400）。
/// 这条同时守住"重建 = 实时"：重启后从落盘转录重建，派发行还得是 user，否则请求又坏掉。
#[test]
pub(crate) fn node_task_is_a_system_line_but_a_user_message() {
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
    let prepared = core
        .prepare_node(&sid, "== 你的任务 ==\n把事做完")
        .expect("准备节点回合");
    let crate::capabilities::conductor::service::Prepared::Run {
        session,
        prefix,
        llm,
        ..
    } = prepared
    else {
        panic!("节点回合该是可以跑的");
    };
    // 节点执行跟随设置里的流式开关（此前写死非流式，节点在界面上永远不逐字出）。
    assert!(llm.stream, "默认设置下节点执行也要流式");
    let dialogue = session.dialogue();
    let last = dialogue.last().expect("注入过任务");
    assert_eq!(last.role, "user", "派发行进上下文是 user 角色：{:?}", last);
    assert!(last.content.contains("把事做完"), "{}", last.content);
    assert!(
        prefix.iter().any(|e| matches!(
            e,
            crate::capabilities::session::api::SessionEvent::Transcript(lines)
                if lines.iter().any(|l| l.system && l.task && l.line.contains("把事做完"))
        )),
        "转录行要带 system + task 标记（界面是系统行，不是用户行）"
    );
    // 唯一装配点发出去的请求里至少有一条 user 消息（协议要求）。
    let msgs = crate::capabilities::collab::domain::engine::assemble(
        "身份",
        None,
        &[],
        false,
        session.dialogue(),
        &[],
    );
    assert!(
        msgs.iter().any(|m| m.role == "user"),
        "请求里必须有 user 消息：{:?}",
        msgs.iter().map(|m| m.role.clone()).collect::<Vec<_>>()
    );
    // 重建 = 实时：重启后从落盘流水重建，最后这条还得是同一条 user 消息。
    core.put_single_recorded(&sid, *session, &prefix);
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    // keep_id = MAX：不截断，只为触发"按落盘转录重建"。
    core2.rewind(&sid, u64::MAX).expect("回档即重建");
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let tail = rebuilt.last().expect("重建后仍有任务行");
    assert_eq!(tail.role, "user", "重建后派发行仍是 user 角色：{:?}", tail);
    assert!(tail.content.contains("把事做完"), "{}", tail.content);
}

#[test]
pub(crate) fn edit_session_writes_meta_appends_config_record_and_rebuilds() {
    let hist = Arc::new(InMemoryHistory::new());
    let mut a = module_of("a");
    a.manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let mut core = core_with_pkgs(
        vec![a, module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert!(
        !core.session_config(&sid).unwrap().started,
        "单 agent 会话在用户开口之前还没内容：名字仍改得动"
    );
    // 改：模块 a → b，模型指定 m，档位换虚拟机档、放行网络。
    // 虚拟机档现在一律不可选（guest 本体尚未接入），所以**先把它做成已存在的虚拟机档会话**——
    // 已在 vm 档的会话只校验它自己那几项（基础根等），改模块/模型/网络照旧允许。
    hist.force_tier(&sid, Tier::Vm);
    let base_dir = crate::tests::scratch("edit-session-base");
    core.edit_session(
        &sid,
        SessionEdit {
            agents: vec![ConfigAgent {
                name: "a".to_string(),
                modules: vec!["b".to_string()],
                model: "m".to_string(),
            }],
            tier: "vm".to_string(),
            base: Some(base_dir.to_string_lossy().into_owned()),
            pins: BTreeMap::new(),
            net: true,
        },
    )
    .unwrap();
    let cfg = core.session_config(&sid).unwrap();
    assert_eq!(cfg.agents[0].modules, vec!["b".to_string()]);
    assert_eq!(cfg.tier, "vm");
    assert!(cfg.net, "网络开关随提交生效");
    let (meta, events) = hist.load(&sid).unwrap();
    assert_eq!(
        meta.agents[0].modules,
        vec!["b".to_string()],
        "meta.yaml 是名单的唯一真相"
    );
    assert_eq!(meta.exec.tier, Tier::Vm);
    assert_eq!(
        meta.exec.base.as_deref(),
        Some(base_dir.to_string_lossy().as_ref()),
        "基础根随提交写回"
    );
    assert!(
        events
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("config")),
        "每次提交编辑追加一条旁路配置记录：{:?}",
        events
    );
    // 内存里的会话按旧配置装过：丢掉后下次访问按新配置从转录重建（内容不丢）。
    assert!(!core.session_exists(&sid));
    // 下一次访问按新配置从转录重建（单 agent 会话还没轮到用户：它只提醒，不硬发请求）。
    with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(core.session_exists(&sid), "访问会话即按新配置重建");
    // 重建后：身份块照样现渲染（不在对话里），对话里的内容也还在。
    let identity = core.single_identity(&sid).expect("重建后身份块");
    assert!(!identity.is_empty(), "重建后身份块还在");
    let _ = core.single_history(&sid).unwrap();
}

#[test]
pub(crate) fn edit_session_freezes_names_only_after_content() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "raw",
        "single",
        vec![agent_meta("a", &["a"], None)],
        ExecSpec::default(),
    );
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    // 没内容：名字与模块都能换。
    core.edit_session("raw", edit_of(vec![("新名", &["b"], "")]))
        .unwrap();
    assert_eq!(core.session_config("raw").unwrap().agents[0].name, "新名");
    // 有内容之后：名字冻结，模块与模型照旧可改。
    hist.append(
        "raw",
        &[serde_json::json!({"type":"transcript","lines":[{"id":0,"line":"[用户] 你好"}]})],
    )
    .unwrap();
    let err = core
        .edit_session("raw", edit_of(vec![("再改", &["b"], "")]))
        .unwrap_err();
    assert!(err.contains("冻结"), "{}", err);
    core.edit_session("raw", edit_of(vec![("新名", &["a"], "m")]))
        .unwrap();
    assert_eq!(
        core.session_config("raw").unwrap().agents[0].modules,
        vec!["a".to_string()]
    );
}

#[test]
pub(crate) fn edit_session_enforces_the_same_rules_as_creation() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], None)],
        ExecSpec::default(),
    );
    seed_session(
        &hist,
        "c",
        "collab",
        vec![agent_meta("a", &["a"], None), agent_meta("b", &["b"], None)],
        ExecSpec::default(),
    );
    let mut core = core_with_pkgs(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let e = core
        .edit_session("w", edit_of(vec![("a", &["没有这个模块"], "")]))
        .unwrap_err();
    assert!(e.contains("无此模块"), "{}", e);
    let e = core
        .edit_session("c", edit_of(vec![("x", &["a"], ""), ("y", &["a"], "")]))
        .unwrap_err();
    assert!(e.contains("只能属于一个 agent"), "{}", e);
    let mut bad_tier = edit_of(vec![("x", &["a"], "")]);
    bad_tier.tier = "docker".to_string();
    assert!(core
        .edit_session("c", bad_tier)
        .unwrap_err()
        .contains("未知执行档位"));
    let mut two_agents = edit_of(vec![("x", &["a"], ""), ("y", &["b"], "")]);
    two_agents.tier = "host".to_string();
    assert!(core
        .edit_session("w", two_agents)
        .unwrap_err()
        .contains("只接受一个 agent"));

    // 虚拟机档选型不成立（同一能力多版本未定版）：编辑与「开始」同一把尺子，如实拒绝。
    let hist2 = Arc::new(InMemoryHistory::new());
    // 已经在虚拟机档上的会话（"留在 vm 档"这一路：只校验它自己那几项，不拿"本机能不能提供 vm 档"拦它）。
    // 基础根给一个真实存在的目录：留在 vm 档时仍然要校验用户填的那个路径。
    let vm_base = crate::tests::scratch("edit-rules-vm-base");
    seed_session(
        &hist2,
        "c",
        "collab",
        vec![agent_meta("x", &["a"], None)],
        ExecSpec {
            tier: Tier::Vm,
            base: Some(vm_base.to_string_lossy().into_owned()),
            ..ExecSpec::default()
        },
    );
    let mut core2 = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist2),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::with(&[
            "id: python\nversion: 3.12.4\nprefix: opt/rt/py312",
            "id: python\nversion: 3.11.9\nprefix: opt/rt/py311",
        ])),
    );
    let mut vm_edit = edit_of(vec![("x", &["a"], "")]);
    vm_edit.tier = "vm".to_string();
    vm_edit.base = Some(vm_base.to_string_lossy().into_owned());
    let e = core2.edit_session("c", vm_edit.clone()).unwrap_err();
    assert!(e.contains("多个版本"), "{}", e);
    // 定版之后可以提交（缺包不拦：那只是该模块工具不可用）。
    vm_edit.pins = BTreeMap::from([("python".to_string(), "3.12.4".to_string())]);
    core2.edit_session("c", vm_edit).unwrap();
    assert_eq!(
        core2
            .session_config("c")
            .unwrap()
            .pins
            .get("python")
            .map(String::as_str),
        Some("3.12.4")
    );
}

#[test]
pub(crate) fn deleting_a_session_asks_the_fence_to_release_its_grants() {
    let hist = Arc::new(InMemoryHistory::new());
    let fence = Arc::new(RecordingFence::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("甲", &["a"], None)],
        ExecSpec::default(),
    );
    let gateway: Arc<dyn ChatGateway + Send + Sync> =
        Arc::new(gw(BTreeMap::new(), vec!["[]".into()]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core = Conductor::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm)),
        test_history_of(Arc::clone(&hist)),
        test_workspace(
            Arc::new(VecSource(vec![module_of("a")])),
            Arc::new(InMemoryPackages::empty()),
            Arc::new(InMemoryWorkspace::new()),
        ),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::clone(&fence)
                as Arc<dyn crate::capabilities::tools::ports::FenceHost + Send + Sync>,
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
    );
    assert!(core.history_delete("w").unwrap(), "会话目录该被删掉");
    assert_eq!(
        fence.released.lock().expect("锁").as_slice(),
        &["甲".to_string()],
        "删除会话要先请适配层撤销该 agent 的围栏授权（痕迹与会话同生共死）"
    );
}

#[test]
pub(crate) fn session_meta_exec_section_roundtrips_and_reads_legacy_meta() {
    let meta = SessionMeta {
        name: "w".to_string(),
        mode: "single".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 1,
        agents: Vec::new(),
        exec: ExecSpec {
            tier: Tier::Vm,
            base: Some("base-linux".to_string()),
            pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
            net: false,
        },
        parent: None,
        node: None,
    };
    let text = serde_yaml::to_string(&meta).expect("序列化");
    let back: SessionMeta = serde_yaml::from_str(&text).expect("反序列化");
    assert_eq!(back.exec.tier, Tier::Vm);
    assert_eq!(back.exec.base.as_deref(), Some("base-linux"));
    assert_eq!(
        back.exec.pins.get("python").map(String::as_str),
        Some("3.12.4")
    );
    assert!(!back.exec.net);
    // 缺 exec 段的旧会话照旧可读（默认 = 本机档、不联网、不定版）。
    let legacy: SessionMeta =
        serde_yaml::from_str("name: old\nmode: single\nmodules: [a]\nts: 1\n")
            .expect("旧 meta.yaml 必须可读");
    assert_eq!(legacy.exec.tier, Tier::Host);
    assert!(legacy.exec.base.is_none());
    assert!(!legacy.exec.net);
    assert!(legacy.exec.pins.is_empty());
}

/// 回档**按回复原子**：截在一次回复中间时整条回复一起丢，绝不留下"孤儿工具结果"。
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
    let replayed = core.rewind(&sid, 2).unwrap();
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

/// 侧栏顺序 = **树序**：父会话紧跟它的子会话（顺序与缩进同源，不会再错位）。
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
