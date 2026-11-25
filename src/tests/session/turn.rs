//! 目的：回合生成与视图：流式上限、讨论轮、单模块、文件视图与工具结果注入。
//! 管：设置对流式的上限、agent 自身历史、核心系统提示、单模块对话、工作区/文件视图、工具结果进入历史。
//! 不管：回档与编辑。
//! 联动：`tests::builders`、`Faces`；契约见 docs/session/session-model.md。

use super::super::builders::*;
use super::super::prelude::*;
use crate::units::conversation::session::api::SINGLE;

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

#[test]
pub(crate) fn core_direct_seeds_system_prompt() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"你好\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    // 回归：单 agent 会话的身份块带能力摘要与工具用法（由会话参数现渲染，不进对话列表）。
    let identity = core.single_identity(&sid).expect("身份块");
    assert!(identity.contains("能力：a 的能力"), "{}", identity);
    assert!(identity.contains("用法：a"), "{}", identity);
    assert!(
        core.single_history(&sid).unwrap().is_empty(),
        "对话里只有真正发生过的事（此刻还没有）"
    );
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "在吗",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
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
        .create_work(work("w", SINGLE, &["a", "b"]))
        .unwrap()
        .sid;
    let meta = core.lifecycle.history_open("w").unwrap().0;
    assert_eq!(meta.spec.kind, SINGLE);
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
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "在吗",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
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
    use crate::units::conversation::session::api::SessionEvent;
    use crate::units::conversation::session::api::{PersistPolicy, is_system_session};
    let line = SessionEvent::Transcript(vec![
        crate::units::conversation::session::api::LineView::system("", "x".to_string()),
    ]);
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
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
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
    manifest_tools.insert("scan".to_string(), decl("python tools/scan.py"));
    let mut mod_a = module_of("a");
    mod_a.manifest.tools = manifest_tools;
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "核对一下",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
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
        rows[1].1.contains("scan") && rows[1].1.contains("成功"),
        "{:?}",
        rows[1]
    );
    assert!(rows[2].1.contains("结论成立"), "{:?}", rows[2]);
    // 历史完整：用户消息 → 工具信封原文 → 工具结果 → 最终答复。
    let h = core.single_history(&sid).unwrap();
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] a.scan")),
        "工具结果必须回注上下文（标签带模块）：{:?}",
        h
    );
    assert!(
        h.iter()
            .any(|m| m.role == "assistant" && m.content.contains("结论成立"))
    );
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}
