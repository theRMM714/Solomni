//! 目的：回合生成与视图：流式上限、讨论轮、单模块、文件视图与工具结果注入。
//! 管：设置对流式的上限、agent 自身历史、核心系统提示、单模块对话、工作区/文件视图、工具结果进入历史。
//! 不管：回档与编辑。
//! 联动：`tests::builders`、`Ops`；契约见 docs/session/session-model.md。

use super::super::builders::*;
use super::super::prelude::*;

/// 目的：设置是流式的**上限**：设置关掉时，调用方要流式也拿不到（全局通用）。
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

/// 目的：用户进 agent 会话说的话，**下一回合它带着**——"讨论与执行不分家"的核心承诺。
/// 约束：判据在"回合收到的消息"上——主会话内容靠**注入**（讨论上下文）给各 agent，而用户在某个 agent 会话说的话进**它自己的历史**，下一回合一起带上。
#[test]
pub(crate) fn discussion_turn_carries_the_agent_sessions_own_history() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut chat = super::super::RecordingChat {
        inner: scripted(vec!["{\"type\":\"say\",\"text\":\"收到\"}".into()]),
        seen: Arc::clone(&seen),
    };
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let turn = crate::capabilities::collab::service::discussion::Discussion::turn_with(
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
    // 回归：单 agent 会话的身份块带能力摘要与工具用法（由会话参数现渲染，不进对话列表）。
    let identity = core.single_identity(&sid).expect("身份块");
    assert!(identity.contains("能力：a 的能力"), "{}", identity);
    assert!(identity.contains("用法：a"), "{}", identity);
    assert!(
        core.single_history(&sid).unwrap().is_empty(),
        "对话里只有真正发生过的事（此刻还没有）"
    );
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    // 用户发言必须入转录：只进历史不进转录的话，历史回放会丢用户消息。
    match &events[0] {
        SessionEvent::Transcript(lines) => {
            assert_eq!(lines[0].kind, "user");
            assert_eq!(lines[0].speaker, "用户");
            assert_eq!(lines[0].line, "在吗");
        }
        _ => panic!("首条应为用户转录行"),
    }
    assert!(
        events.iter().any(
            |e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.speaker == "a"))
        )
    );
}

#[test]
pub(crate) fn single_mode_accepts_multi_module_agent_and_converses() {
    // 单 agent 形态允许 1 个 agent 带多个模块：能建、能力与用法并入全部模块、说话人是 agent 名。
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
        identity.contains("能力：a 的能力")
            && identity.contains("能力：b 的能力")
            && identity.contains("用法：a")
            && identity.contains("用法：b"),
        "身份块必须并入全部模块的能力与用法：{}",
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

/// 目的：**落盘策略由会话种类定、判定只有一处**：短暂事件任何会话都不留；系统会话（`#` 开头）一条都不留。
#[test]
pub(crate) fn persist_policy_is_decided_by_the_session_kind() {
    use crate::capabilities::conductor::service::persist::{PersistPolicy, is_system_session};
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
    assert!(
        h.iter()
            .any(|m| m.role == "assistant" && m.content.contains("结论成立"))
    );
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}

/// 目的：**派发行：界面是系统行，上下文是 user 角色**——Web 与 CLI 走同一条语义。
/// 约束：界面上它是核心说的话（不得显示成"用户"），但请求里必须有一条 user 消息；一条 user 都没有的请求会被供应商**整条拒收**（真机 400）。这条同时守住"重建 = 实时"：重启后从落盘转录重建，派发行还得是 user，否则请求又坏掉。
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
        identity,
        prefix,
        task,
        llm,
        ..
    } = prepared
    else {
        panic!("节点回合该是可以跑的");
    };
    // 节点执行跟随设置里的流式开关；任务由工作线程在压缩后注入。
    assert!(llm.stream, "默认设置下节点执行也要流式");
    let task = task.expect("节点回合带任务");
    let mut session = session;
    let events = std::cell::RefCell::new(prefix);
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut live = Live {
            llm,
            cancel,
            emit: &mut |ev| events.borrow_mut().push(ev),
            decisions: None,
            ask: None,
        };
        let mut sink = |ev| events.borrow_mut().push(ev);
        crate::capabilities::collab::api::dispatch_task(
            &mut session,
            &task,
            &identity,
            &mut live,
            &mut sink,
        );
    }
    let events = events.into_inner();
    let dialogue = session.dialogue();
    let task_msg = dialogue
        .iter()
        .rev()
        .find(|m| m.role == "user" && m.content.contains("把事做完"))
        .expect("注入过任务");
    assert_eq!(
        task_msg.role, "user",
        "派发行进上下文是 user 角色：{:?}",
        task_msg
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            crate::capabilities::session::api::SessionEvent::Transcript(lines)
                if lines.iter().any(|l| l.system && l.task && l.line.contains("把事做完"))
        )),
        "转录行要带 system + task 标记（界面是系统行，不是用户行）"
    );
    // 唯一装配点发出去的请求里至少有一条 user 消息（协议要求）。
    let msgs = crate::capabilities::collab::service::round::assemble(
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
    core.put_single_recorded(&sid, *session, &events);
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
    core2
        .rewind(
            &sid,
            crate::capabilities::conductor::api::RewindTarget::Delete(u64::MAX),
        )
        .expect("回档即重建");
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let task_msg = rebuilt
        .iter()
        .rev()
        .find(|m| m.role == "user" && m.content.contains("把事做完"))
        .expect("重建后仍有任务行");
    assert_eq!(
        task_msg.role, "user",
        "重建后派发行仍是 user 角色：{:?}",
        task_msg
    );
}
