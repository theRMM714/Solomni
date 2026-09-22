//! 核心测试（T1）：全内存装配，不碰文件系统。
//! 替身与装配辅助见 super::doubles；层级与判定见 docs/testing/levels.md。

use super::doubles::*;
use crate::adapters::fake_chat::FakeChat;
use crate::core::engine::{
    Discussion, Member, MemberTools, ModuleTools, TurnOut, MAX_ROUNDS, MAX_TOOL_CALLS,
};
use crate::core::events::Live;
use crate::core::exec::{self, Diagnosis, ExecSpec, Tier};
use crate::core::history::{AgentMeta, SessionMeta};
use crate::core::module::Module;
use crate::core::packages::{Library, PackageManifest};
use crate::core::ports::{
    BoxedChat, Chat, ChatGateway, Chunk, CompleteOpts, Completion, HistoryStore, ModuleSource, Msg,
    ToolOutcome, ToolRunner, Workspace,
};
use crate::core::prompt::render;
use crate::core::providers::{Channel, ModelEntry, Provider, Settings};
use crate::core::{
    AgentInstance, CollabStep, ConfigAgent, Core, Pending, SessionEdit, SessionEvent, WorkMode,
    WorkSpec,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
// ---------- 信封 ----------

#[test]
pub(crate) fn envelope_parse_clean() {
    let r = crate::core::envelope::parse("{\"type\":\"ask\",\"text\":\"要 A 还是 B？\"}");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Ask));
    assert_eq!(r.text, "要 A 还是 B？");
    assert!(!r.degraded);
}

#[test]
pub(crate) fn envelope_degraded_keeps_raw() {
    let r = crate::core::envelope::parse("这不是 JSON");
    assert!(r.degraded);
    assert_eq!(r.text, "这不是 JSON");
}

#[test]
pub(crate) fn envelope_wrapped_json_still_parses() {
    let r = crate::core::envelope::parse("好的：{\"type\":\"agree\",\"text\":\"同意\"} 以上。");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Agree));
    assert!(!r.degraded);
}

#[test]
pub(crate) fn envelope_text_may_be_omitted() {
    let r = crate::core::envelope::parse("{\"type\":\"agree\"}");
    assert!(
        matches!(r.verb, crate::core::envelope::Verb::Agree),
        "缺 text 不影响表态"
    );
    assert_eq!(r.text, "", "缺 text = 空串");
    assert!(!r.degraded);
    let say = crate::core::envelope::parse("{\"type\":\"say\"}");
    assert!(
        matches!(say.verb, crate::core::envelope::Verb::Say) && !say.degraded,
        "缺 text 的发言仍是干净信封"
    );
    assert!(say.text.is_empty());
    // 缺 name 的工具信封仍是 malformed 信号，不被缺省 text 收编成发言。
    let bad = crate::core::envelope::parse("{\"type\":\"tool\",\"args\":{}}");
    assert!(matches!(bad.verb, crate::core::envelope::Verb::Tool));
    assert!(bad.tools.iter().any(|t| t.malformed.is_some()));
}

// ---------- 提示词渲染层 ----------

#[test]
pub(crate) fn prompt_render_replaces_and_rejects_missing() {
    let ok = render("你好 {{name}}！", &[("name", "世界".to_string())]).unwrap();
    assert_eq!(ok, "你好 世界！");
    assert!(render("{{missing}}", &[]).is_err());
}

#[test]
pub(crate) fn prompt_book_loads_from_yaml() {
    let p = test_prompts();
    // 语气约定；**能用哪些表态由角色表渲染**（见 systools/roles.yaml），不在这句话里。
    assert!(!p.core.chat_protocol.trim().is_empty());
    assert!(p.core.discuss.opener.contains("{{protocol}}"));
}

#[test]
pub(crate) fn prompt_render_keeps_single_braces() {
    let ok = render("输出 {\"a\":1} 和 {{x}}", &[("x", "Y".to_string())]).unwrap();
    assert_eq!(ok, "输出 {\"a\":1} 和 Y");
}

// ---------- 登记处解析链与密钥治理 ----------

#[test]
pub(crate) fn settings_resolves_model_to_channel() {
    let mut s = Settings::default();
    s.providers.insert(
        "p".into(),
        Provider {
            base_url: "http://x".into(),
            api_key: "k".into(),
        },
    );
    s.models.insert(
        "m".into(),
        ModelEntry {
            name: "展示名".into(),
            api_model: "real-model".into(),
            provider: "p".into(),
            note: String::new(),
            tools: crate::core::providers::ToolMode::Native,
        },
    );
    s.core = Some("m".into());
    let ch = s.resolve("m").unwrap();
    assert_eq!(
        ch.model, "real-model",
        "发给供应商的是 api_model，不是展示名"
    );
    assert_eq!(ch.provider.base_url, "http://x");
    // 缺省 = envelope（手写信封：任何供应商都能用）；模型视图也如实带出来
    s.models.insert(
        "d".into(),
        ModelEntry {
            name: "缺省".into(),
            api_model: "d".into(),
            provider: "p".into(),
            note: String::new(),
            tools: Default::default(),
        },
    );
    assert!(s.resolve("d").is_ok(), "缺省形态的模型照样能解析出通道");
    assert_eq!(
        s.model_views()
            .iter()
            .find(|v| v.id == "m")
            .map(|v| v.tools),
        Some(crate::core::providers::ToolMode::Native),
        "模型视图要带上形态（前端显示与探测结果都靠它）"
    );
    assert!(s.resolve("ghost").is_err(), "未知模型必须报错");
    assert_eq!(s.core_channel().unwrap().model, "real-model");
}

#[test]
pub(crate) fn provider_lifecycle_and_key_never_leaks_to_view() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.provider_upsert("p1", "http://x", "sk-密钥XYZ")
        .unwrap();
    for v in core.provider_views() {
        // 展示文案由呈现层拼（core 不再提供 CLI 行），"密钥永不出现"这条红线两处都要成立。
        let shown = format!("{}  {}", v.id, v.base_url);
        assert!(!shown.contains("sk-密钥XYZ"), "视图出现密钥：{}", shown);
        assert!(!format!("{:?}", v).contains("sk-密钥XYZ"));
    }
    // 仍被模型引用时拒绝删除供应商（不静默级联）
    core.model_upsert("m1", "M", "api-m", "p1", "快").unwrap();
    assert!(core
        .provider_remove("p1")
        .unwrap_err()
        .contains("仍被模型引用"));
    assert!(core.model_remove("m1").unwrap());
    assert!(core.provider_remove("p1").unwrap());
}

#[test]
pub(crate) fn model_guards_core_default_and_discovery_uses_stored_provider() {
    let catalog = Arc::new(FakeCatalog::new(vec!["m-a".to_string(), "m-b".to_string()]));
    let mut core = core_with_catalog(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::clone(&catalog),
    );
    core.provider_upsert("p2", "http://x", "k").unwrap();
    assert_eq!(
        core.discover_models("p2").unwrap(),
        vec!["m-a".to_string(), "m-b".to_string()]
    );
    assert_eq!(catalog.seen.lock().expect("锁")[0].base_url, "http://x");
    assert!(core
        .discover_models("ghost")
        .unwrap_err()
        .contains("无此供应商"));
    // 引用了不存在的供应商 → 拒绝登记
    assert!(core
        .model_upsert("bad", "B", "b", "ghost", "")
        .unwrap_err()
        .contains("无此供应商"));
    // 核心默认模型不可删；换默认后旧的可删
    assert!(core.model_remove("m").unwrap_err().contains("核心默认模型"));
    core.model_upsert("m2", "M2", "api-m2", "p2", "").unwrap();
    assert!(core.core_set_model("m2").unwrap());
    assert!(core.model_remove("m").unwrap());
}

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
            SessionEvent::Transcript(l) => l.first().map(|x| x.line.clone()),
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

/// 从事件流里抽出转录行，便于断言。
pub(crate) fn replay_lines(events: &[serde_json::Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
        .flat_map(|e| {
            e.get("lines")
                .and_then(|l| l.as_array())
                .cloned()
                .unwrap_or_default()
        })
        .map(|l| {
            l.get("line")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

/// 事件流里的转录行视图：(id, line, 是否是 tool 行)。
pub(crate) fn transcript_rows(events: &[SessionEvent]) -> Vec<(u64, String, bool)> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .map(|l| (l.id, l.line.clone(), l.tool.is_some()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// 事件流里的工具调用视图（tool 行携带的那个）。
pub(crate) fn tool_views(events: &[SessionEvent]) -> Vec<crate::core::events::ToolCallView> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => {
                ls.iter().filter_map(|l| l.tool.clone()).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect()
}

/// 事件流里 tool 行的行文本（给人看的那一行）。
pub(crate) fn tool_line_texts(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| match e {
            SessionEvent::Transcript(ls) => ls
                .iter()
                .filter(|l| l.tool.is_some())
                .map(|l| l.line.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
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
        3,
        "system + 用户 + 答一"
    );

    // 回档到第 0 行 = 转录清空、历史只剩 system
    let replayed = core.rewind(&sid, 0).unwrap();
    assert!(
        replay_lines(&replayed).is_empty(),
        "点第一行 → 转录清空：{:?}",
        replayed
    );
    assert_eq!(
        core.single_history(&sid).unwrap().len(),
        1,
        "历史只剩 system"
    );

    // next_line 归零：下一条行 id 从 0 开始
    let ev = with_live(|l| core.single_say(&sid, "再来", l)).unwrap();
    let first = transcript_rows(&ev).first().cloned().expect("应有新行");
    assert_eq!(first.0, 0, "回档清空后行 id 从头计");
}

#[test]
pub(crate) fn rebuilt_context_keeps_tool_result() {
    // 同一份落盘历史 + 新的 Core 模拟「重启」：重建上下文时工具子轮不能丢。
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
pub(crate) fn collab_state_derive_and_withdraw() {
    use crate::core::collab_state::derive;
    let ev = |id: u64, line: &str| serde_json::json!({"type":"transcript","lines":[{"id":id,"line":line}]});
    let events = vec![
        ev(0, "[用户:需求] 做个东西"),
        ev(1, "[用户:开始] yes"),
        ev(2, "[轮次 2]"),
        ev(3, "[a:agree] 同意"),
    ];
    let st = derive(&events, &["a".to_string()]);
    assert_eq!(st.task.as_deref(), Some("做个东西"));
    assert!(st.begun && !st.allow);
    assert_eq!(st.round, 2);
    assert!(st.agreed["a"] && st.closed, "全员同意即收敛");
    // 撤回该同意后：不再算同意、讨论不再收敛
    let mut withdrawn = events.clone();
    withdrawn.push(ev(4, "[用户:撤回] a"));
    let st2 = derive(&withdrawn, &["a".to_string()]);
    assert!(!st2.agreed["a"] && !st2.closed);
    // 代拟行只给人看：名单不由它派生（权威来源是 meta.agents）。
    let with_slate = vec![
        ev(0, "[代拟] 甲〈a〉→ m（对口）；乙（复用；补位）"),
        ev(1, "[用户:名单] 确认"),
    ];
    let st3 = derive(&with_slate, &["a".to_string()]);
    assert_eq!(st3.picked, vec!["a".to_string()], "名单仍来自 meta.agents");
    assert!(
        st3.slate.is_some() && st3.slate_confirmed,
        "只保留原文供展示"
    );
}

#[test]
pub(crate) fn collab_rewind_rebuilds_from_transcript_and_resume_waits_at_gate() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(collab_work("c", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    // 回档到需求行之后（保留前 1 行 = 需求行）：协作走「按转录重建」
    let replayed = core.rewind(&sid, 1).unwrap();
    assert_eq!(
        replay_lines(&replayed),
        vec!["[用户:需求] 做个东西".to_string()]
    );
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))),
        "重建后仍等确认开始"
    );
    // 继续：未开始不由继续代劳，只提醒
    let ev = with_live(|l| core.continue_flow(&sid, l)).unwrap();
    assert!(matches!(&ev[0], SessionEvent::Notice(n) if n.contains("裁决门")));
}

#[test]
pub(crate) fn collab_update_task_rewinds_and_latest_wins() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(collab_work("c", &["a"], false, "旧需求"))
        .unwrap()
        .sid;
    let replayed = core.update_task(&sid, "新需求").unwrap();
    assert_eq!(
        replay_lines(&replayed),
        vec![
            "[用户:需求] 旧需求".to_string(),
            "[用户:需求] 新需求".to_string()
        ]
    );
    // 派生以最后一条需求为准
    let (_, events) = core.history_open("c").unwrap();
    let st = crate::core::collab_state::derive(&events, &["a".to_string()]);
    assert_eq!(st.task.as_deref(), Some("新需求"));
}

#[test]
pub(crate) fn suggest_models_recommends_agents() {
    // 不存在的模块 / 不存在的模型 / 重复占用的模块一律拒收。
    let script = "{\"agents\":[{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"},{\"name\":\"乙\",\"modules\":[\"b\"],\"model\":\"m\",\"why\":\"补位\"},{\"name\":\"鬼\",\"modules\":[\"ghost\"],\"model\":\"m\",\"why\":\"模块不存在\"},{\"name\":\"丙\",\"modules\":[\"a\"],\"model\":\"nope\",\"why\":\"模型不存在\"},{\"name\":\"丁\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"重复占模块\"}]}".to_string();
    let core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec![script.clone()]),
    );
    // 协作：两个独立 agent，各带自己的模块与模型。
    let collab = core.suggest_models("做个东西", WorkMode::Collab).unwrap();
    assert_eq!(collab.len(), 2);
    assert_eq!(collab[0].name, "甲");
    assert_eq!(collab[0].modules, vec!["a".to_string()]);
    assert_eq!(collab[1].modules, vec!["b".to_string()]);
    assert_eq!(collab[0].model, "m");
    assert_eq!(collab[0].why, "对口");
    // 单 agent：多条推荐 → 并成一个临时 agent（并过的不是任何单个已存 agent，故 reuse=false）。
    let merged = core.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].modules, vec!["a".to_string(), "b".to_string()]);
    assert!(!merged[0].reuse, "并出来的 agent 不是复用项");
}

#[test]
pub(crate) fn suggest_models_single_mode_keeps_lone_pick_as_is() {
    // 只给一条 → 原样采纳（模块数与 reuse 都保持它自己的，不裁模块、不改 reuse）。
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec![
        "{\"agents\":[{\"name\":\"全能\",\"modules\":[\"a\",\"b\"],\"model\":\"m\",\"why\":\"一个 AI 全包\"}]}".to_string(),
    ]));
    let out = core.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, "全能");
    assert_eq!(
        out[0].modules,
        vec!["a".to_string(), "b".to_string()],
        "单 agent 不再被裁到一个模块"
    );
    assert!(!out[0].reuse);
    // 复用项也要原样保留 reuse 标记
    let mut reuse = core_with(
        vec![module_of("a")],
        gw(
            BTreeMap::new(),
            vec!["{\"agents\":[{\"agent\":\"调研\",\"why\":\"正好\"}]}".to_string()],
        ),
    );
    reuse
        .agent_upsert("调研", &["a".to_string()], "m", "")
        .unwrap();
    let got = reuse.suggest_models("做个东西", WorkMode::Single).unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].reuse, "一条复用项必须保留 reuse");
    assert_eq!(got[0].model, "m");
}

#[test]
pub(crate) fn suggest_models_reuses_stored_agent_without_suggesting_model() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        // 只有复用项（模型与模块都取登记处自己的）；幽灵项应被拒收。
        "{\"agents\":[{\"agent\":\"调研\",\"why\":\"正好用得上\"},{\"agent\":\"幽灵\",\"why\":\"不在登记处\"}]}".to_string(),
    ]));
    core.agent_upsert("调研", &["a".to_string()], "m", "说明")
        .unwrap();
    let out = core.suggest_models("做个东西", WorkMode::Collab).unwrap();
    assert_eq!(out.len(), 1, "非法的复用项必须被拒收");
    assert!(out[0].reuse, "复用项要如实标记");
    assert_eq!(out[0].name, "调研");
    assert_eq!(out[0].modules, vec!["a".to_string()]);
    assert_eq!(out[0].model, "m", "模型取登记处里那个，核心不代拟");
    // 全部拒收 → 明确报错（不悄悄给个空名单）
    let mut empty = core_with(
        vec![module_of("a")],
        gw(
            BTreeMap::new(),
            vec!["{\"agents\":[{\"agent\":\"幽灵\",\"why\":\"不在登记处\"}]}".to_string()],
        ),
    );
    empty
        .agent_upsert("调研", &["a".to_string()], "m", "")
        .unwrap();
    assert!(empty
        .suggest_models("做个东西", WorkMode::Collab)
        .unwrap_err()
        .contains("没有可用结果"));
}

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

#[test]
pub(crate) fn agent_crud_and_work_with_agents() {
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    assert!(core
        .agent_upsert("", &["a".to_string()], "", "")
        .unwrap_err()
        .contains("不能为空"));
    assert!(core
        .agent_upsert("x", &[], "", "")
        .unwrap_err()
        .contains("至少要有一个模块"));
    assert!(core
        .agent_upsert("x", &["ghost".to_string()], "", "")
        .unwrap_err()
        .contains("无此模块"));
    assert!(core
        .agent_upsert("x", &["a".to_string()], "nope", "")
        .unwrap_err()
        .contains("无此模型"));
    core.agent_upsert("调研", &["a".to_string(), "b".to_string()], "m", "说明")
        .unwrap();
    assert_eq!(core.agent_views().len(), 1);
    assert_eq!(core.agent_views()[0].modules.len(), 2);

    // 组合：一个 agent（多模块合并）
    let spec = WorkSpec {
        name: "w".to_string(),
        mode: WorkMode::Single,
        agents: vec![AgentInstance {
            name: "调研".to_string(),
            transient: false,
            modules: vec!["a".to_string(), "b".to_string()],
            model: Some("m".to_string()),
        }],
        task: None,
        delegate: false,
    };
    let opened = core.create_work(spec).unwrap();
    assert_eq!(opened.agents, vec!["调研".to_string()]);
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研");

    // 协作：多 agent；同工作内重名自动加尾号
    let mut dup = collab_work("c", &["a", "b"], false, "需求");
    dup.agents[1].name = dup.agents[0].name.clone();
    let o = core.create_work(dup).unwrap();
    assert_eq!(o.agents, vec!["a".to_string(), "a-2".to_string()]);

    // 同一模块不得跨 agent 重复
    let cross = WorkSpec {
        name: "cross".to_string(),
        mode: WorkMode::Collab,
        agents: vec![
            AgentInstance {
                name: "x".to_string(),
                transient: true,
                modules: vec!["a".to_string()],
                model: None,
            },
            AgentInstance {
                name: "y".to_string(),
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

    assert!(core.agent_remove("调研").unwrap());
    assert!(core.agent_views().is_empty());
}

#[test]
pub(crate) fn work_upload_conflict_and_safe_name() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(work("up", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    assert!(core.work_upload(&sid, "x.txt", b"hello", false).unwrap());
    assert!(
        !core.work_upload(&sid, "x.txt", b"again", false).unwrap(),
        "同名不覆盖"
    );
    assert!(
        core.work_upload(&sid, "x.txt", b"again", true).unwrap(),
        "显式覆盖"
    );
    assert!(core
        .work_upload("ghost", "x.txt", b"x", false)
        .unwrap_err()
        .contains("无此会话"));
    assert!(core
        .work_upload(&sid, "../evil.txt", b"x", false)
        .unwrap_err()
        .contains("路径分隔符"));
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
    // 单 agent 形态只接受一个 agent（模块数不限，多模块合法，见 single_mode_accepts_multi_module_agent）
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
    assert!(core
        .create_work(two_agents)
        .unwrap_err()
        .contains("只接受一个 agent"));
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

#[test]
pub(crate) fn model_catalog_parses_openai_shape_and_rejects_bad() {
    use crate::adapters::model_catalog::parse_models;
    assert_eq!(
        parse_models(r#"{"data":[{"id":"gpt-4o"},{"id":"o3"},{"id":"gpt-4o"}]}"#).unwrap(),
        vec!["gpt-4o".to_string(), "o3".to_string()]
    );
    assert!(
        parse_models(r#"{"models":["a"]}"#).is_err(),
        "缺 data 必须报错"
    );
    assert!(parse_models(r#"{"data":[]}"#).is_err(), "空列表必须报错");
    assert!(parse_models("不是 JSON").is_err());
}

#[test]
pub(crate) fn endpoint_candidates_complete_and_fall_back() {
    use crate::adapters::endpoint::{chat_candidates, models_candidates, retryable_status};
    // 已带版本段：只补后缀（含尾斜杠）
    assert_eq!(
        chat_candidates("https://api.x/v1"),
        vec!["https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        chat_candidates("https://api.x/v1/"),
        vec!["https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        chat_candidates("https://api.x/v2"),
        vec!["https://api.x/v2/chat/completions"]
    );
    assert_eq!(
        models_candidates("https://api.x/v1"),
        vec!["https://api.x/v1/models"]
    );
    // 无版本段：先直连，连不上再回落 /v1
    assert_eq!(
        chat_candidates("https://api.x"),
        vec![
            "https://api.x/chat/completions",
            "https://api.x/v1/chat/completions"
        ]
    );
    assert_eq!(
        models_candidates("https://api.x"),
        vec!["https://api.x/models", "https://api.x/v1/models"]
    );
    // 已是完整端点：原样，防重复拼接
    assert_eq!(
        chat_candidates("https://api.x/v1/chat/completions"),
        vec!["https://api.x/v1/chat/completions"]
    );
    assert_eq!(
        models_candidates("https://api.x/v1/models"),
        vec!["https://api.x/v1/models"]
    );
    // 换候选判定：只有 404/405 换，鉴权类立即报
    assert!(retryable_status(404) && retryable_status(405));
    assert!(
        !retryable_status(401)
            && !retryable_status(403)
            && !retryable_status(429)
            && !retryable_status(500)
    );
}

#[test]
pub(crate) fn endpoint_resolve_candidates_retries_shape_mismatch_and_stops_on_fatal() {
    use crate::adapters::endpoint::{resolve_candidates, Attempt};
    let cands = vec!["a".to_string(), "b".to_string(), "c".to_string()];

    // 回归：SPA catch-all 返回 200 HTML（形状不符=Retry）时必须换到下一个候选，而不是立即报错。
    let mut seen = Vec::new();
    let got = resolve_candidates(
        &cands,
        |url| {
            seen.push(url.to_string());
            if url == "b" {
                Attempt::Ok(7)
            } else {
                Attempt::Retry("响应不是 JSON".to_string())
            }
        },
        |_, _, _| {},
    );
    assert_eq!(got.unwrap(), ("b".to_string(), 7));
    assert_eq!(seen, vec!["a".to_string(), "b".to_string()]);

    // 鉴权类 Fatal 立即停，不再试后续候选。
    let mut seen2 = Vec::new();
    let fatal: Result<(String, i32), String> = resolve_candidates(
        &cands,
        |url| {
            seen2.push(url.to_string());
            Attempt::Fatal("供应商返回 401".to_string())
        },
        |_, _, _| {},
    );
    assert_eq!(fatal.unwrap_err(), "供应商返回 401");
    assert_eq!(seen2, vec!["a".to_string()]);

    // 全部 Retry 耗尽：报最后一个错误，不静默。
    let all: Result<(String, i32), String> =
        resolve_candidates(&cands, |_| Attempt::Retry("失败".to_string()), |_, _, _| {});
    assert_eq!(all.unwrap_err(), "失败");
}

// ---------- 模块清单（内存来源） ----------

#[test]
pub(crate) fn roster_lists_modules() {
    let core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    let r = core.scan();
    let ids: Vec<_> = r.modules.iter().map(|m| m.manifest.id.clone()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}

// ---------- 讨论引擎 ----------

// ---------- 出站调用的全局参数（流式 / 预算 / 失败中断） ----------

/// 记录调用参数、并能按脚本失败的通道替身。
/// 用来钉两件机器可判的事实：**流式与预算来自全局设置**、**调用失败 = 中断而不是发言**。
pub(crate) struct OptsChat {
    pub(crate) seen: OptsLog,
    /// 每次调用的结果：None = 回一个 agree 信封；Some(原因) = 失败。
    pub(crate) results: Vec<Option<String>>,
}

/// 调用参数账本：(流式, 预算秒)。
pub(crate) type OptsLog = Arc<Mutex<Vec<(bool, u64)>>>;

impl crate::core::ports::Chat for OptsChat {
    fn complete(
        &mut self,
        _m: &[crate::core::ports::Msg],
        opts: crate::core::ports::CompleteOpts<'_>,
        _on: &mut dyn FnMut(crate::core::ports::Chunk) -> bool,
    ) -> crate::core::ports::Completion {
        self.seen
            .lock()
            .expect("锁")
            .push((opts.stream, opts.timeout_secs));
        let r = if self.results.len() > 1 {
            self.results.remove(0)
        } else {
            self.results.first().cloned().unwrap_or(None)
        };
        match r {
            Some(reason) => crate::core::ports::Completion::failure(reason),
            // 默认回**发言**而不是同意：同意是粘住的，开场就同意会让后面几轮被跳过，
            // 那些用例（预算 / 失败中断）要的是"还在讨论中"。
            None => crate::core::ports::Completion::text("{\"type\":\"say\",\"text\":\"我先说\"}"),
        }
    }
}

fn opts_discussion(
    results: Vec<Option<String>>,
    llm: crate::core::ports::LlmOpts,
) -> (Discussion, OptsLog) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let members = vec![Member::new(
        "m0",
        "职责0".to_string(),
        Box::new(OptsChat {
            seen: Arc::clone(&seen),
            results,
        }),
    )];
    (
        Discussion::new(
            members,
            false,
            test_prompts(),
            llm,
            Default::default(),
            String::new(),
            vec![],
        ),
        seen,
    )
}

/// 讨论的调用参数**必须来自全局设置**（以前这里写死非流式，正是协作卡住的成因之一）。
#[test]
pub(crate) fn discussion_calls_carry_the_global_streaming_and_budget() {
    let llm = crate::core::ports::LlmOpts {
        stream: true,
        timeout_secs: 123,
    };
    let (mut d, seen) = opts_discussion(vec![None, None, None], llm);
    assert!(
        d.open("任务", &mut |_, _| {}, &mut |_| {}).is_ok(),
        "开场正常"
    );
    let got = seen.lock().expect("锁").clone();
    assert_eq!(
        got[0],
        (true, 123),
        "开场调用要带上设置里的流式与预算：{:?}",
        got
    );
    let _ = d.step(&mut |_, _| {}, &mut |_| {});
    let got = seen.lock().expect("锁").clone();
    assert_eq!(got[1], (true, 123), "轮次调用同样：{:?}", got);
}

/// 调用失败：如实中断，**绝不把失败当发言吸收**（错误文本一旦进转录，"轮到谁"就歪了）。
#[test]
pub(crate) fn discussion_call_failure_interrupts_without_absorbing_a_line() {
    let (mut d, _seen) = opts_discussion(
        vec![None, Some("模型调用失败：超时".to_string()), None],
        Default::default(),
    );
    assert!(
        d.open("任务", &mut |_, _| {}, &mut |_| {}).is_ok(),
        "开场正常"
    );
    match d.step(&mut |_, _| {}, &mut |_| {}) {
        TurnOut::Interrupted(err) => assert!(err.contains("超时"), "原因要原样带回：{}", err),
        other => panic!(
            "失败必须中断，实际：{}",
            match other {
                TurnOut::Round => "Round",
                TurnOut::Done => "Done",
                TurnOut::AskUser { .. } => "AskUser",
                TurnOut::Interrupted(_) => "Interrupted",
                TurnOut::Stopped => "Stopped",
            }
        ),
    }
    // 开场那一次是正常的（留下一条发言）；失败那一次**不该**再添发言。
    let spoken = d
        .transcript
        .iter()
        .filter(|l| l.text.contains("[m0:"))
        .count();
    assert_eq!(
        spoken,
        1,
        "失败不该被当成发言（只有开场那一条）：{:?}",
        d.transcript
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        d.transcript.iter().all(|l| !l.text.contains("超时")),
        "失败原因不该进转录"
    );
    assert!(!d.closed, "中断后讨论保持可继续（用户点「继续」重试）");
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

pub(crate) fn scripted_discussion(scripts: Vec<Vec<String>>, allow: bool) -> Discussion {
    let members: Vec<Member> = scripts
        .into_iter()
        .enumerate()
        .map(|(i, s)| Member::new(&format!("m{}", i), format!("职责{}", i), scripted(s)))
        .collect();
    Discussion::new(
        members,
        allow,
        test_prompts(),
        Default::default(),
        Default::default(),
        String::new(),
        vec![],
    )
}

/// 链随 plan_review 事件派生：按转录重建时**不重新整理**（省一次模型调用）。
#[test]
pub(crate) fn collab_state_derives_the_task_chain_from_plan_review() {
    let events = vec![
        serde_json::json!({ "type": "plan", "text": "方案：A 做 X" }),
        serde_json::json!({
            "type": "plan_review",
            "plan": "方案：A 做 X",
            "chain": { "nodes": [ {
                "id": "n1", "title": "做 X", "objective": "把 X 做完",
                "assignee": "a", "deps": []
            } ] }
        }),
    ];
    let st = crate::core::collab_state::derive(&events, &["a".to_string()]);
    assert_eq!(st.plan.as_deref(), Some("方案：A 做 X"));
    assert_eq!(st.chain.nodes.len(), 1, "链该从 plan_review 派生出来");
    assert_eq!(st.chain.nodes[0].assignee, "a");
    assert_eq!(st.chain.nodes[0].objective, "把 X 做完");
}

// ---------- 任务链（依赖图） ----------

fn chain_node(id: &str, deps: &[&str]) -> crate::core::chain::TaskNode {
    crate::core::chain::TaskNode {
        id: id.to_string(),
        title: format!("节点{}", id),
        objective: format!("把 {} 做完", id),
        assignee: "甲".to_string(),
        deps: deps.iter().map(|d| d.to_string()).collect(),
        status: crate::core::chain::NodeStatus::Pending,
        sub_session: None,
        acceptance: None,
    }
}

fn roster() -> Vec<String> {
    vec!["甲".to_string(), "乙".to_string()]
}

/// 串：链式依赖按序就绪——前一环没完成，后一环不开始。
#[test]
pub(crate) fn chain_ready_advances_along_a_serial_chain() {
    use crate::core::chain::NodeStatus;
    let mut chain = crate::core::chain::TaskChain {
        nodes: vec![
            chain_node("a", &[]),
            chain_node("b", &["a"]),
            chain_node("c", &["b"]),
        ],
    };
    assert_eq!(chain.ready().len(), 1);
    assert_eq!(chain.ready()[0].id, "a");
    chain.nodes[0].status = NodeStatus::Done;
    assert_eq!(chain.ready()[0].id, "b");
    chain.nodes[1].status = NodeStatus::Done;
    assert_eq!(chain.ready()[0].id, "c");
    chain.nodes[2].status = NodeStatus::Done;
    assert!(chain.ready().is_empty(), "全完成之后没有可启动的");
    assert!(chain.finished());
}

/// 并 + 混合：无依赖的节点**同时**就绪；"等它们全部结束"的节点在两者都完成后才就绪。
#[test]
pub(crate) fn chain_ready_returns_parallel_nodes_together() {
    use crate::core::chain::NodeStatus;
    let mut chain = crate::core::chain::TaskChain {
        nodes: vec![
            chain_node("a", &[]),
            chain_node("b", &[]),
            chain_node("c", &["a", "b"]),
        ],
    };
    let mut ids: Vec<String> = chain.ready().iter().map(|n| n.id.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["a".to_string(), "b".to_string()],
        "a/b 该同时就绪"
    );
    assert!(chain.ready().iter().all(|n| n.id != "c"), "c 要等 a、b");
    chain.nodes[0].status = NodeStatus::Done;
    assert_eq!(chain.ready().len(), 1, "只完成一半，c 还不能开始");
    assert_eq!(chain.ready()[0].id, "b");
    chain.nodes[1].status = NodeStatus::Done;
    assert_eq!(chain.ready()[0].id, "c", "两个依赖都完成，c 就绪");
    assert!(!chain.finished(), "c 还没结束");
}

/// 装配期自洽：环、悬空依赖、重复 id、空目标、未知负责人——逐条如实列出。
#[test]
pub(crate) fn chain_problems_reject_cycles_and_bad_refs() {
    use crate::core::chain::TaskChain;
    let good = TaskChain {
        nodes: vec![chain_node("a", &[]), chain_node("b", &["a"])],
    };
    assert!(
        good.problems(&roster()).is_empty(),
        "{:?}",
        good.problems(&roster())
    );

    // 环：a 等 b、b 等 a。
    let cyc = TaskChain {
        nodes: vec![chain_node("a", &["b"]), chain_node("b", &["a"])],
    };
    let p = cyc.problems(&roster());
    assert!(p.iter().any(|x| x.contains("环")), "{:?}", p);

    // 悬空依赖 + 重复 id + 空目标 + 未知负责人。
    let mut bad = TaskChain {
        nodes: vec![chain_node("a", &["没有这个"]), chain_node("a", &[])],
    };
    bad.nodes[1].objective = String::new();
    bad.nodes[1].assignee = "丙".to_string();
    let p = bad.problems(&roster());
    assert!(p.iter().any(|x| x.contains("不存在的节点")), "{:?}", p);
    assert!(p.iter().any(|x| x.contains("id 重复")), "{:?}", p);
    assert!(p.iter().any(|x| x.contains("没有目标")), "{:?}", p);
    assert!(p.iter().any(|x| x.contains("不在名单里")), "{:?}", p);

    // 空链也算装配错误（没什么可推进的）。
    assert!(!TaskChain::default().problems(&roster()).is_empty());
}

/// 结束判定：链非空、且每个节点都落定（Done / Failed）——空链不算结束。
#[test]
pub(crate) fn chain_finished_needs_every_node_settled() {
    use crate::core::chain::{NodeStatus, TaskChain};
    let mut chain = TaskChain {
        nodes: vec![chain_node("a", &[]), chain_node("b", &["a"])],
    };
    assert!(!chain.finished());
    chain.nodes[0].status = NodeStatus::Done;
    assert!(!chain.finished(), "b 还没落定");
    // 失败也算落定（链不静默跳过：会暂停并通知用户，但不会永远卡着）。
    chain.nodes[1].status = NodeStatus::Failed;
    assert!(chain.finished());
    assert!(!TaskChain::default().finished(), "空链不算结束");
}

/// 原生通道：供应商的结构化槽位 → 讨论动词；不认识的工具名 = 不认识（调用点据此**如实拒绝**）。
#[test]
pub(crate) fn native_tool_names_map_to_discussion_verbs() {
    use crate::core::engine::{arg_text, verb_of};
    use crate::core::envelope::Verb;
    assert_eq!(verb_of("say"), Some(Verb::Say));
    assert_eq!(verb_of("agree"), Some(Verb::Agree));
    assert_eq!(verb_of("leave"), Some(Verb::Leave));
    assert_eq!(verb_of("ask"), Some(Verb::Ask));
    // 不是协作动词 = 不认识：讨论里出现这种调用就是越权，调用点会如实拒绝、不当表态吸收。
    assert_eq!(verb_of("read"), None);
    assert_eq!(verb_of("create_session"), None);
    // 参数里取正文；取不到就是空串（不猜）。
    assert_eq!(arg_text("{\"text\":\"同意\"}"), "同意");
    assert_eq!(arg_text("{}"), "");
    assert_eq!(arg_text("不是 JSON"), "");
}

/// 同意是**粘住**的：发过 agree 的人不再被追问（以前每轮重置，等于每轮把所有人问一遍）。
#[test]
pub(crate) fn agreement_is_sticky_so_agreed_members_are_not_asked_again() {
    let mut d = scripted_discussion(
        vec![
            vec!["{\"type\":\"agree\",\"text\":\"同意\"}".into()],
            vec![
                "{\"type\":\"say\",\"text\":\"我补充\"}".into(),
                "{\"type\":\"say\",\"text\":\"再补充\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
            vec![
                "{\"type\":\"say\",\"text\":\"我也说\"}".into(),
                "{\"type\":\"say\",\"text\":\"还说\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
        ],
        false,
    );
    assert!(
        d.open("任务", &mut |_, _| {}, &mut |_| {}).is_ok(),
        "开场正常"
    );
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    // m0 在开场就同意了：之后每一轮都该跳过它（以前每轮重置同意，会把它反复问一遍）。
    let asked_m0 = d
        .transcript
        .iter()
        .filter(|l| l.text.contains("[m0:"))
        .count();
    assert_eq!(
        asked_m0,
        1,
        "发过 agree 的人不该被再问一次：{:?}",
        d.transcript
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
pub(crate) fn discussion_full_agreement() {
    let mut d = scripted_discussion(
        vec![
            vec![
                "{\"type\":\"say\",\"text\":\"好\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
            vec![
                "{\"type\":\"say\",\"text\":\"行\"}".into(),
                "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
            ],
        ],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.text.contains("[m0:agree]")));
}

#[test]
pub(crate) fn discussion_ask_pauses() {
    let mut d = scripted_discussion(
        vec![vec!["{\"type\":\"ask\",\"text\":\"需要参数?\"}".into()]],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    match d.step(&mut |_, _| {}, &mut |_| {}) {
        TurnOut::AskUser { member, question } => {
            assert_eq!(member, "m0");
            assert_eq!(question, "需要参数?");
        }
        _ => panic!("应暂停请教"),
    }
}

#[test]
pub(crate) fn discussion_leave_shrinks() {
    let mut d = scripted_discussion(
        vec![vec!["{\"type\":\"leave\",\"text\":\"撤了\"}".into()]],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    let _ = d.step(&mut |_, _| {}, &mut |_| {});
    assert!(d.members.iter().all(|m| !m.present));
}

#[test]
pub(crate) fn discussion_autonomy_archives_ask() {
    let mut d = scripted_discussion(
        vec![vec![
            "{\"type\":\"say\",\"text\":\"开场\"}".into(),
            "{\"type\":\"ask\",\"text\":\"细节?\"}".into(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
        ]],
        true,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("自裁模式不该暂停"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.text.contains("自裁")));
}

#[test]
pub(crate) fn discussion_round_cap_enforced() {
    let mut d = scripted_discussion(
        vec![vec!["{\"type\":\"say\",\"text\":\"继续\"}".into()]; 2],
        false,
    );
    let _ = d.open("任务", &mut |_, _| {}, &mut |_| {});
    loop {
        match d.step(&mut |_, _| {}, &mut |_| {}) {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
            TurnOut::Interrupted(e) => panic!("不该中断：{e}"),
            TurnOut::Stopped => panic!("不该停止"),
        }
    }
    assert!(d.round > MAX_ROUNDS);
}

#[test]
pub(crate) fn degraded_discussion_line_carries_a_structured_flag() {
    let prompts = test_prompts();
    // 成员给出"不是信封"的原文 → 该行按降级收录：文本里有说明，**结构上另带 degraded**。
    let members = vec![Member::new(
        "m0",
        "职责".to_string(),
        scripted(vec!["我觉得可以".into()]),
    )];
    let mut disc = Discussion::new(
        members,
        true,
        prompts.clone(),
        Default::default(),
        Default::default(),
        String::new(),
        vec![],
    );
    let _ = disc.open("任务", &mut |_, _| {}, &mut |_| {});
    let line = disc
        .transcript
        .iter()
        .find(|l| l.text.starts_with("[m0:say]"))
        .expect("应有 m0 的发言行");
    assert!(
        line.degraded,
        "降级必须带结构化标记（呈现层靠它，不靠匹配文案）"
    );
    assert!(
        line.text
            .contains(&prompts.core.tool_texts.discuss_degraded),
        "文本里仍保留给人/模型看的说明"
    );

    // 线格式：只在为真时写出 degraded
    let yes = SessionEvent::Transcript(vec![crate::core::events::LineView {
        id: 0,
        line: "x".into(),
        degraded: true,
        ..Default::default()
    }])
    .to_json();
    assert_eq!(yes["lines"][0]["degraded"], serde_json::Value::Bool(true));
    let no = SessionEvent::Transcript(vec![crate::core::events::LineView {
        id: 0,
        line: "x".into(),
        ..Default::default()
    }])
    .to_json();
    assert!(
        no["lines"][0].get("degraded").is_none(),
        "非降级行不写这个字段"
    );
}

// ---------- 执行/验收 ----------

#[test]
pub(crate) fn execution_review_pass_and_fail_paths() {
    let prompts = test_prompts();
    let mut members = vec![Member::new(
        "m0",
        "职责".to_string(),
        scripted(vec!["{\"type\":\"say\",\"text\":\"汇报内容\"}".into()]),
    )];
    let mut exec = crate::core::engine::Execution::run(
        members.as_mut_slice(),
        "任务A",
        &prompts,
        Default::default(),
    );
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("汇报内容"));
    let mut core_chat = scripted(vec![
        "[{\"item\":\"A\",\"status\":\"fail\",\"reason\":\"没做完\"}]".into(),
    ]);
    exec.review(core_chat.as_mut(), "方案", &prompts, Default::default());
    assert!(!exec.all_pass());
    exec.rerun(members.as_mut_slice(), "任务A", "- A：没做完", &prompts);
    let mut core_chat2 = scripted(vec!["[{\"item\":\"A\",\"status\":\"pass\"}]".into()]);
    exec.review(core_chat2.as_mut(), "方案", &prompts, Default::default());
    assert!(exec.all_pass());
}

#[test]
pub(crate) fn review_parse_failure_is_conservative_fail() {
    let prompts = test_prompts();
    let mut members = vec![Member::new(
        "m0",
        "职责".to_string(),
        scripted(vec!["{\"type\":\"say\",\"text\":\"x\"}".into()]),
    )];
    let mut exec = crate::core::engine::Execution::run(
        members.as_mut_slice(),
        "任务",
        &prompts,
        Default::default(),
    );
    let mut core_chat = scripted(vec!["完全不是清单".to_string()]);
    exec.review(core_chat.as_mut(), "方案", &prompts, Default::default());
    assert!(exec.items.is_empty());
    assert!(!exec.all_pass(), "解析失败必须保守判否");
}

// ---------- Core 门面：会话中心（内存组合根） ----------

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
    // 回归：单 agent 会话的历史首条必须是职责提示词（system）。
    let h = core.single_history(&sid).unwrap();
    assert_eq!(h[0].role, "system");
    assert!(h[0].content.contains("你负责a"));
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    // 回归：用户发言必须入转录（此前只进历史、不进转录，历史回放会丢用户消息）。
    match &events[0] {
        SessionEvent::Transcript(lines) => assert_eq!(lines[0].line, "[用户] 在吗"),
        _ => panic!("首条应为用户转录行"),
    }
    assert!(events.iter().any(
        |e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("[a]")))
    ));
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
    let h = core.single_history(&sid).unwrap();
    assert_eq!(h[0].role, "system");
    assert!(
        h[0].content.contains("你负责a") && h[0].content.contains("你负责b"),
        "system 必须并入全部模块职责"
    );
    let events = with_live(|l| core.single_say(&sid, "在吗", l)).unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.starts_with("[组合] ")))),
        "转录说话人必须是 agent 名：{:?}",
        events
    );
}

#[test]
pub(crate) fn mode_vocabulary_is_single_or_collab_only() {
    // web 与 core 的形态词汇只有 single / collab；旧的 direct / compose 一概不认（GREEN FIELD，无兼容）。
    use crate::presentation::web::parse_mode;
    assert!(matches!(parse_mode("single"), Ok(WorkMode::Single)));
    assert!(matches!(parse_mode("collab"), Ok(WorkMode::Collab)));
    for bad in ["direct", "compose", "omni", ""] {
        let e = parse_mode(bad).unwrap_err();
        assert!(e.contains("未知模式"), "web 必须 400 并说明：{}", e);
    }
    // 落盘 meta 里的旧形态不做兼容：重建会话时明确报错（不静默当单 agent）。
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
    );
    hist.create(&SessionMeta {
        name: "旧会话".to_string(),
        mode: "direct".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 1,
        agents: vec![AgentMeta {
            name: "a".to_string(),
            transient: true,
            modules: vec!["a".to_string()],
            model: None,
        }],
        exec: ExecSpec::default(),
        parent: None,
        node: None,
    })
    .unwrap();
    // 内存里没有这个会话 → 走 rebuild_session，对未知形态如实报错。
    let err = core.rewind("旧会话", 0).unwrap_err();
    assert!(err.contains("未知会话形态"), "{}", err);
}

/// 方案过审后：就绪节点各建一个**子会话**（普通单 agent 会话，沙箱锚在父会话上）。
#[test]
pub(crate) fn approved_plan_spawns_a_sub_session_per_ready_node() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}".to_string(),
                "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    core.collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    let after = core
        .collab_continue(&sid, CollabStep::ApprovePlan, "")
        .unwrap();

    let started: Vec<(String, String, String)> = after
        .iter()
        .filter_map(|e| match e {
            SessionEvent::NodeStarted {
                node,
                sid,
                assignee,
            } => Some((node.clone(), sid.clone(), assignee.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(started.len(), 1, "一个就绪节点该建一个子会话：{:?}", after);
    let (node, child, assignee) = &started[0];
    assert_eq!(node, "n1");
    assert_eq!(assignee, "a");
    assert_eq!(child, &format!("{}--n1", sid));

    // 子会话是**普通单 agent 会话**：meta 记着编排者与节点，沙箱锚在父会话上。
    let (cmeta, _) = core.history_open(child).unwrap();
    assert_eq!(cmeta.parent.as_deref(), Some(sid.as_str()));
    assert_eq!(cmeta.node.as_deref(), Some("n1"));
    assert_eq!(cmeta.mode, "single");
    assert_eq!(cmeta.work(), sid.as_str(), "沙箱锚在父会话上（共用工作区）");
    assert_eq!(cmeta.agents.len(), 1, "子会话只有一个席位");
    assert_eq!(cmeta.agents[0].name, "a");
}

/// 审查关卡：整理完**不自动开工**——停在待审，点「同意」才推进（见 docs/architecture/task-chain.md）。
#[test]
pub(crate) fn collab_pauses_for_plan_review_until_the_user_approves() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意方案\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}".to_string(),
                "[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;

    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::PlanReview { .. })),
        "整理完该停在**待审**：{:?}",
        events
    );
    assert!(
        !events.iter().any(|e| matches!(
            e,
            SessionEvent::Report { .. } | SessionEvent::Delivery { .. }
        )),
        "没过审不该开工：{:?}",
        events
    );
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::PlanReview))),
        "待审要挂起等用户"
    );
    // 链随方案一起交给用户审查：节点、负责人、目标都在（审查关卡看的就是这张图）。
    let reviewed = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::PlanReview { chain, .. } => Some(chain.clone()),
            _ => None,
        })
        .expect("待审事件要带链");
    assert_eq!(reviewed.nodes.len(), 1, "链里该有一个节点：{:?}", reviewed);
    assert_eq!(reviewed.nodes[0].assignee, "a");
    assert_eq!(reviewed.nodes[0].objective, "把 X 做完");

    // 点「同意」之后才推进：执行回报与交付都该出现。
    let after = core
        .collab_continue(&sid, CollabStep::ApprovePlan, "")
        .unwrap();
    assert!(
        after
            .iter()
            .any(|e| matches!(e, SessionEvent::Report { .. })),
        "同意后该开工：{:?}",
        after
    );
    assert!(
        after
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { .. })),
        "同意后该走到交付：{:?}",
        after
    );
}

#[test]
pub(crate) fn core_collab_demo_runs_full_five_stages() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意方案\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}".to_string(),
                "[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a"], false, "做个东西"))
        .unwrap()
        .sid;
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(
            core.collab_continue(&sid, CollabStep::ApprovePlan, "")
                .unwrap(),
        );
        e
    };
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Plan(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
    // 终结会话已被中心回收（再查询挂起状态应报无此会话）。
    assert!(core.collab_pending(&sid).is_err());
}

#[test]
pub(crate) fn core_collab_delegated_slate_flow() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        // 代拟（组装一个 agent）→ 整理 → 验收。
        "{\"picks\":[{\"name\":\"a\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}".to_string(),
        "{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
    ]));
    let opened = core
        .create_work(collab_work("w", &[], true, "做个东西"))
        .unwrap();
    let sid = opened.sid;
    let ev = opened.events;
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmSlate))
    ));
    assert!(ev.iter().any(
        |e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("[代拟]")))
    ));
    let _ = core
        .collab_continue(&sid, CollabStep::ConfirmSlate, "yes")
        .unwrap();
    assert!(matches!(
        core.collab_pending(&sid),
        Ok(Some(Pending::ConfirmBegin))
    ));
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(
            core.collab_continue(&sid, CollabStep::ApprovePlan, "")
                .unwrap(),
        );
        e
    };
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
}

#[test]
pub(crate) fn core_collab_slate_rejects_invalid_picks() {
    // 不存在的模块 / 不存在的模型 / 不在登记处的复用项：整条拒收，合法的留下。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"picks\":[{\"name\":\"鬼\",\"modules\":[\"ghost\"],\"model\":\"m\",\"why\":\"模块不存在\"},{\"agent\":\"幽灵\",\"why\":\"不在登记处\"},{\"name\":\"甲\",\"modules\":[\"a\"],\"model\":\"nope\",\"why\":\"模型不存在\"},{\"name\":\"乙\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案\"}".to_string(),
        "[{\"item\":\"x\",\"status\":\"pass\"}]".to_string(),
    ]));
    let ev = core
        .create_work(collab_work("w", &[], true, "任务"))
        .unwrap()
        .events;
    for want in ["ghost 不存在", "幽灵 不在登记处", "nope 不存在"] {
        assert!(
            ev.iter()
                .any(|e| matches!(e, SessionEvent::Notice(n) if n.contains(want))),
            "应如实拒收：{}",
            want
        );
    }
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.line.contains("乙〈a〉→ m")))));
}

#[test]
pub(crate) fn collab_delegated_roster_written_back_and_rebuilt_from_meta() {
    // 发言席是 agent：脚本按 agent 名 "调研员" 回放（不再按模块 id）。
    let mut member = BTreeMap::new();
    member.insert(
        "调研员".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        "{\"picks\":[{\"name\":\"调研员\",\"modules\":[\"a\"],\"model\":\"m\",\"why\":\"对口\"}]}".to_string(),
        // 负责人必须是**名单里真实存在的席位**（代拟出来的叫"调研员"）——否则链的自洽门禁会如实挡下。
        "{\"plan\":\"方案：A 做 X\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"调研员\",\"deps\":[]}]}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
    ]));
    let sid = core
        .create_work(collab_work("w", &[], true, "做个东西"))
        .unwrap()
        .sid;
    assert!(
        core.history_open("w").unwrap().0.agents.is_empty(),
        "确认名单之前不落档（名单只活在内存里）"
    );
    // CLI 的确认门要能把这份表单逐行读出来。
    let slate = core.collab_slate(&sid).unwrap();
    assert_eq!(slate.len(), 1);
    assert_eq!(slate[0].name, "调研员");
    assert!(slate[0].transient, "组装项如实标记为临时 agent");
    assert_eq!(slate[0].model.as_deref(), Some("m"));

    core.collab_continue(&sid, CollabStep::ConfirmSlate, "yes")
        .unwrap();
    // 确认后名单写回 meta（重启/回档后的权威来源）。
    let meta = core.history_open("w").unwrap().0;
    assert_eq!(meta.agents.len(), 1);
    assert_eq!(meta.agents[0].name, "调研员");
    assert_eq!(meta.agents[0].modules, vec!["a".to_string()]);
    assert_eq!(meta.agents[0].model.as_deref(), Some("m"));
    assert_eq!(meta.modules, vec!["a".to_string()]);

    // 回档（保留需求行）= 按 meta.agents 重建会话（协作走「按转录重建」这条路），再跑到交付。
    core.rewind(&sid, 1).unwrap();
    assert!(
        matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))),
        "重建后仍等确认开始"
    );
    let events = core
        .collab_continue(&sid, CollabStep::Begin, "yes")
        .unwrap();
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    let events = {
        let mut e = events;
        e.extend(
            core.collab_continue(&sid, CollabStep::ApprovePlan, "")
                .unwrap(),
        );
        e
    };
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "重建出来的名单要能一路跑完：{:?}",
        events
    );
}

// ---------- 平衡提取器 ----------

#[test]
pub(crate) fn extract_balanced_array() {
    let s = "前缀 [ {\"a\":1}, {\"b\":\"}\"} ] 后缀";
    let got = crate::core::envelope::extract_json_array(s).unwrap();
    assert!(got.starts_with('[') && got.ends_with(']'));
    let obj = crate::core::envelope::extract_json_object("x {\"k\":\"{\"} y").unwrap();
    assert!(obj.starts_with('{') && obj.ends_with('}'));
}

// ---------- 工具执行器 ----------

/// 守护 runner：任何调用即失败（守护不该用工具的路径）。
pub(crate) struct SilentRunner;
impl ToolRunner for SilentRunner {
    fn run(
        &self,
        _fence: &crate::core::fence::FenceSpec,
        _command: &str,
        _args: &str,
    ) -> ToolOutcome {
        panic!("不应调用工具");
    }
}

/// 记录型 runner：记录 (root, command, args)，回放固定输出。
pub(crate) struct RecordingRunner {
    pub(crate) calls: Mutex<Vec<(PathBuf, String, String)>>,
    out: String,
    ok: bool,
}
impl RecordingRunner {
    pub(crate) fn new(out: &str, ok: bool) -> RecordingRunner {
        RecordingRunner {
            calls: Mutex::new(Vec::new()),
            out: out.to_string(),
            ok,
        }
    }
}
impl ToolRunner for RecordingRunner {
    fn run(
        &self,
        fence: &crate::core::fence::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome {
        // 记下工具进程的工作目录（= 该模块的根）与命令、参数。
        self.calls.lock().expect("锁").push((
            fence.cwd.clone(),
            command.to_string(),
            args_json.to_string(),
        ));
        ToolOutcome {
            ok: self.ok,
            output: self.out.clone(),
        }
    }
}

/// 并发记录型 runner：记录**同时在跑**的调用数峰值（模块工具是否真的并发，只有它能作证）。
pub(crate) struct ParallelRunner {
    active: AtomicUsize,
    peak: AtomicUsize,
    delay_ms: u64,
}

impl ParallelRunner {
    pub(crate) fn new(delay_ms: u64) -> ParallelRunner {
        ParallelRunner {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay_ms,
        }
    }

    /// 同时在跑的峰值（串行恒为 1）。
    pub(crate) fn peak_concurrent(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

impl ToolRunner for ParallelRunner {
    fn run(
        &self,
        _fence: &crate::core::fence::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if self.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.delay_ms));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        ToolOutcome {
            ok: true,
            output: format!("{} 跑完了 {}", command, args_json),
        }
    }
}

const TOOL_CALL: &str = "{\"type\":\"tool\",\"name\":\"grep\",\"args\":{\"keyword\":\"x\"}}";

/// 带工具环境的成员：模块 m0 声明 grep → python tools/grep.py（cwd = 该模块目录）。
pub(crate) fn member_with_tools(
    id: &str,
    script: Vec<String>,
    runner: Arc<impl ToolRunner + Send + Sync + 'static>,
) -> Member {
    let mut commands = BTreeMap::new();
    commands.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut modules = BTreeMap::new();
    modules.insert(
        "m0".to_string(),
        ModuleTools {
            root: abs(&["mods", "root"]),
            commands,
            books: BTreeMap::new(),
            parallel: BTreeSet::new(),
        },
    );
    let mut m = Member::new(id, "职责".to_string(), scripted(script));
    // 该路径走模块声明的外部命令（grep）：空沙箱 + 内存 IO，内置工具不参与。
    m.tools = Some(MemberTools {
        mode: crate::core::providers::ToolMode::Envelope,
        modules,
        observations: crate::core::systool::Observations::default(),
        repair: Arc::new(NoRepair),
        log: Arc::new(crate::core::ports::NoopLog),
        runner,
        sandbox: test_sandbox("m0", &[]),
        io: Arc::new(InMemorySysIo::new()),
        unavailable: BTreeMap::new(),
        fence: crate::core::fence::FenceSpec::from_sandbox(&test_sandbox("m0", &[]), false),
        reply_seq: 0,
        llm: Default::default(),
    });
    m
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
    // 提示词里的工具清单按模块分组，模型照此写 module。
    let h = core.single_history(&sid).unwrap();
    assert!(
        h[0].content.contains("- a：read_txt") && h[0].content.contains("- b：read_txt"),
        "清单要按模块分组：{}",
        h[0].content
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
pub(crate) fn refs_rewrite_covers_prefixes_speakers_and_punctuation() {
    use crate::core::refs::{rewrite, RefRoots};
    // 文案来自提示词册：期望值也用册子渲染出来，代码里不复制那两句中文。
    let t = test_prompts().core.refs;
    let foreign = |path: &str, agent: &str| {
        crate::core::prompt::render(
            &t.foreign_sandbox,
            &[("agent", agent.to_string()), ("path", path.to_string())],
        )
        .expect("册子变量齐全")
    };
    let collab = |path: &str, agent: &str| {
        crate::core::prompt::render(
            &t.collab_sandbox,
            &[("path", path.to_string()), ("agent", agent.to_string())],
        )
        .expect("册子变量齐全")
    };
    assert!(
        t.foreign_sandbox.contains("{{agent}}") && t.foreign_sandbox.contains("{{path}}"),
        "无权文案要同时报出文件与 agent"
    );
    // 真实根：共享区 + 本 agent（甲）的私有沙箱
    let roots = RefRoots {
        work: abs(&["w", "work"]),
        private: Some(abs(&["w", "甲"])),
    };
    let collab_roots = RefRoots {
        work: abs(&["w", "work"]),
        private: None,
    };
    // @work：与 speaker 无关，一律给出共享区真实路径
    assert_eq!(
        rewrite("@work:a.txt", Some("甲"), &roots, &t),
        s(&["w", "work", "a.txt"])
    );
    assert_eq!(
        rewrite("@work:sub/a.txt", None, &roots, &t),
        s(&["w", "work", "sub", "a.txt"])
    );
    // @sandbox：命中自己的沙箱 → 私有沙箱真实路径
    assert_eq!(
        rewrite("@sandbox:甲/note.txt", Some("甲"), &roots, &t),
        s(&["w", "甲", "note.txt"])
    );
    // @sandbox：别人的沙箱 → 册子文案（带文件名与 agent 名，不泄漏真实路径）
    assert_eq!(
        rewrite("@sandbox:乙/note", Some("甲"), &roots, &t),
        foreign("note", "乙")
    );
    // @sandbox：协作（没有"自己的沙箱"）→ 册子里的协作文案
    assert_eq!(
        rewrite("@sandbox:乙/note", None, &collab_roots, &t),
        collab("note", "乙")
    );
    // 终止标点留在原文里（只替换前缀+路径），所以句中标点/收尾标点都原样保留
    assert_eq!(
        rewrite("@work:a.txt，", Some("甲"), &roots, &t),
        format!("{}，", s(&["w", "work", "a.txt"]))
    );
    assert_eq!(
        rewrite("看 @sandbox:甲/b.txt。", Some("甲"), &roots, &t),
        format!("看 {}。", s(&["w", "甲", "b.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt 请读它", Some("甲"), &roots, &t),
        format!("{} 请读它", s(&["w", "work", "a.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt，请读它", Some("甲"), &roots, &t),
        format!("{}，请读它", s(&["w", "work", "a.txt"]))
    );
    // 句点不是终止符：note.txt 是完整路径
    assert_eq!(
        rewrite("@work:note.txt", Some("甲"), &roots, &t),
        s(&["w", "work", "note.txt"])
    );
    assert_eq!(
        rewrite("@sandbox:乙/note.txt", Some("甲"), &roots, &t),
        foreign("note.txt", "乙"),
        "扩展名要算进路径，不能截成 note"
    );
    assert_eq!(
        rewrite("@sandbox:乙/\"note.txt\"", Some("甲"), &roots, &t),
        foreign("note.txt", "乙")
    );
    // 已知取舍：句尾英文句点算进路径（要精确表达就用引号形式）
    assert_eq!(
        rewrite("@work:a.txt.", Some("甲"), &roots, &t),
        s(&["w", "work", "a.txt."])
    );
    assert_eq!(
        rewrite("@work:\"a.txt.\"", Some("甲"), &roots, &t),
        s(&["w", "work", "a.txt."])
    );
    // 引号形式：空白与标点都算路径的一部分
    assert_eq!(
        rewrite("@work:\"项目 说明.md\"", Some("甲"), &roots, &t),
        s(&["w", "work", "项目 说明.md"])
    );
    assert_eq!(
        rewrite("@work:\"a,b(1).md\"", Some("甲"), &roots, &t),
        s(&["w", "work", "a,b(1).md"])
    );
    assert_eq!(
        rewrite("@sandbox:甲/\"a b.md\"", Some("甲"), &roots, &t),
        s(&["w", "甲", "a b.md"])
    );
    assert_eq!(
        rewrite("@work:\"a b.md\" 看一下", Some("甲"), &roots, &t),
        format!("{} 看一下", s(&["w", "work", "a b.md"]))
    );
    // agent 名也支持引号（名字含空白）：引号后必须紧跟 /
    assert_eq!(
        rewrite(
            "@sandbox:\"调研 助手\"/\"a b.md\"",
            Some("调研 助手"),
            &roots,
            &t
        ),
        s(&["w", "甲", "a b.md"])
    );
    assert_eq!(
        rewrite("@sandbox:\"调研 助手\"/a", Some("甲"), &roots, &t),
        foreign("a", "调研 助手")
    );
    // 根还没就绪（代拟确认前）：原样保留引用，不编路径
    assert_eq!(
        rewrite("@work:a.txt", None, &RefRoots::default(), &t),
        "@work:a.txt"
    );
    // 不完整前缀 / 结构不满足 / 引号未闭合：原样输出（不猜）
    assert_eq!(rewrite("@work:", Some("甲"), &roots, &t), "@work:");
    assert_eq!(rewrite("@work: ", Some("甲"), &roots, &t), "@work: ");
    assert_eq!(
        rewrite("@sandbox:甲", Some("甲"), &roots, &t),
        "@sandbox:甲",
        "缺相对路径"
    );
    assert_eq!(
        rewrite("@sandbox:/a.txt", Some("甲"), &roots, &t),
        "@sandbox:/a.txt",
        "缺 agent 名"
    );
    assert_eq!(
        rewrite("@sandbox:甲/", Some("甲"), &roots, &t),
        "@sandbox:甲/",
        "相对路径为空"
    );
    assert_eq!(
        rewrite("@work:\"a b.md", Some("甲"), &roots, &t),
        "@work:\"a b.md",
        "路径引号未闭合"
    );
    assert_eq!(
        rewrite("@work:\"\"", Some("甲"), &roots, &t),
        "@work:\"\"",
        "空引号路径"
    );
    assert_eq!(
        rewrite("@sandbox:\"调研 助手/a.md", Some("甲"), &roots, &t),
        "@sandbox:\"调研 助手/a.md",
        "agent 名引号未闭合"
    );
    assert_eq!(
        rewrite("@sandbox:\"调研\"x/a.md", Some("甲"), &roots, &t),
        "@sandbox:\"调研\"x/a.md",
        "agent 名引号后缺 /"
    );
    // 普通文本 / 误伤：不含两种前缀一律原样
    assert_eq!(rewrite("", Some("甲"), &roots, &t), "");
    assert_eq!(rewrite("没有引用", Some("甲"), &roots, &t), "没有引用");
    assert_eq!(
        rewrite("email@xxx.com 是我的", Some("甲"), &roots, &t),
        "email@xxx.com 是我的"
    );
    assert_eq!(
        rewrite("@workx:a.txt", Some("甲"), &roots, &t),
        "@workx:a.txt"
    );
    // 一句里多个引用；引号形式与无引号旧形式并存
    assert_eq!(
        rewrite("@work:a.txt 和 @sandbox:甲/b.txt", Some("甲"), &roots, &t),
        format!(
            "{} 和 {}",
            s(&["w", "work", "a.txt"]),
            s(&["w", "甲", "b.txt"])
        )
    );
    assert_eq!(
        rewrite("@work:a.txt 与 @work:\"a b.md\"", Some("甲"), &roots, &t),
        format!(
            "{} 与 {}",
            s(&["w", "work", "a.txt"]),
            s(&["w", "work", "a b.md"])
        )
    );
}

#[test]
pub(crate) fn user_at_reference_is_rewritten_in_transcript_and_history() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好的\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "@work:a.txt 看一下", l)).unwrap();
    // 转录的用户行已是确切寻址（不再是 @ 引用）
    let rows = transcript_rows(&events);
    let want = format!("[用户] {} 看一下", s(&["w", "work", "a.txt"]));
    assert_eq!(rows[0].1, want, "{:?}", rows);
    // 进上下文的是同一份文本（转录即内容）
    let h = core.single_history(&sid).unwrap();
    let want_msg = format!("{} 看一下", s(&["w", "work", "a.txt"]));
    assert!(
        h.iter().any(|m| m.role == "user" && m.content == want_msg),
        "{:?}",
        h
    );
    assert!(
        !h.iter().any(|m| m.content.contains("@work:")),
        "历史里不得残留 @ 引用：{:?}",
        h
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

/// 真实的坏信封：结尾多了一个 ]（括号不配对）。路径用真实绝对路径（模型写对了路径、写坏了信封）。
/// 坏的形状照真案例：args 先闭合、多一个 ]、外层才闭合（整段不是合法 JSON）。
pub(crate) fn broken_tool(path: &str) -> String {
    format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}]}}",
        path
    )
}

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
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "写个 README", l)).unwrap();
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
    assert_eq!(views[0].raw, broken, "原文留档（重建上下文用）");
    assert_eq!(
        io.get(&["w", "work", "README.md"]),
        None,
        "非法信封绝不执行工具"
    );
    // 历史：assistant(原文) + [工具结果]（含册子文案）→ 模型下一轮能自己改
    let h = core.single_history(&sid).unwrap();
    assert!(h
        .iter()
        .any(|m| m.role == "assistant" && m.content == broken));
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
    core2.rewind(&sid, 4).unwrap();
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&rebuilt), key(&h), "重建上下文必须与实时历史逐条一致");
}

#[test]
pub(crate) fn malformed_tool_without_salvageable_name_still_records_a_line() {
    // 断在半截、括号不平衡：打捞不到名字也不能 panic，仍是一条 ok=false 的 tool 行。
    let half = "{\"type\":\"tool\",\"args\":{";
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            half.to_string(),
            "{\"type\":\"say\",\"text\":\"知道了\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "跑一下", l)).unwrap();
    let views = tool_views(&events);
    assert_eq!(views.len(), 1, "仍要记一条：{:?}", transcript_rows(&events));
    assert!(
        !views[0].ok && views[0].name.is_empty(),
        "打捞不到名字就留空：{:?}",
        views[0]
    );
    assert!(
        !transcript_rows(&events)
            .iter()
            .any(|r| r.1.contains("\"type\"")),
        "JSON 不进文本"
    );
}

#[test]
pub(crate) fn repeated_malformed_envelopes_hit_the_tool_cap_and_stop() {
    // 模型反复输出非法信封：计入工具上限，最终强制收尾，不会死循环。
    let broken = broken_tool(&s(&["w", "work", "README.md"]));
    let script: Vec<String> = (0..MAX_TOOL_CALLS + 1).map(|_| broken.clone()).collect();
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "一直错", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        MAX_TOOL_CALLS,
        "上限内每次非法信封各记一条：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 不进文本：{:?}",
        rows
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
pub(crate) fn prose_then_unclosed_tool_envelope_is_malformed_and_keeps_prose() {
    // 正文在前、坏信封在后（未闭合）：也要判 malformed，且正文照常显示、JSON 不上屏。
    let raw = "好的。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\"}";
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            raw.to_string(),
            "{\"type\":\"say\",\"text\":\"改好了\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "写一下", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        1,
        "应有一条 tool 行：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 绝不能上屏：{:?}",
        rows
    );
    assert!(
        rows.iter().any(|r| r.1 == "[a] 好的。"),
        "坏信封之前的正文要照常显示：{:?}",
        rows
    );
    let views = tool_views(&events);
    assert!(!views[0].ok && views[0].name == "write", "{:?}", views[0]);
    assert!(
        views[0].args.contains("\"path\":\"a\""),
        "参数尽力打捞：{:?}",
        views[0].args
    );
    assert!(
        views[0].output.contains("还差 }"),
        "未闭合要说清还差哪个字符：{}",
        views[0].output
    );
    // 历史：assistant(原文) + [工具结果]（模型下一轮能自己改）
    let h = core.single_history(&sid).unwrap();
    assert!(h.iter().any(|m| m.role == "assistant" && m.content == raw));
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "{:?}",
        h
    );
}

#[test]
pub(crate) fn a_malformed_envelope_is_repaired_when_the_fix_is_unambiguous() {
    // 真实事故的形状：write 的 content 里直接换了行 → 手写信封非法。
    // 默认修复器只做无歧义的转义；修好就照常执行，并在回执最前面如实标注。
    let note = s(&["demo", "m0", "note.txt"]);
    let raw = format!(
        "{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"第一行\n第二行\"}}}}",
        note
    );
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: String::new(),
        ok: true,
    });
    let prompts = test_prompts();
    // 不修（严格基线）：信封不合法 → 失败工具行，工具绝不执行
    let mut m = member_with_tools(
        "m0",
        vec![
            raw.clone(),
            "{\"type\":\"say\",\"text\":\"改好了\"}".to_string(),
        ],
        Arc::clone(&runner),
    );
    assert!(m
        .tools
        .as_ref()
        .expect("工具环境")
        .repair
        .repair("x", &crate::core::envelope::Malformed::Syntax("x".into()))
        .repaired
        .is_none());
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
    let trace = exec.traces.get("m0").expect("工具行");
    assert!(!trace[0].ok, "不修时如实记失败：{}", trace[0].output);
    // 默认修复器：同一个输入被无歧义修好 → 照常执行，且回执最前面如实标注
    let mut m2 = member_with_tools(
        "m0",
        vec![
            raw.clone(),
            "{\"type\":\"say\",\"text\":\"改好了\"}".to_string(),
        ],
        Arc::clone(&runner),
    );
    m2.tools.as_mut().expect("工具环境").repair = Arc::new(crate::adapters::UnambiguousRepair);
    let exec2 = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m2),
        "任务",
        &prompts,
        Default::default(),
    );
    let trace2 = exec2.traces.get("m0").expect("工具行");
    assert!(trace2[0].ok, "修好即执行：{}", trace2[0].output);
    assert!(
        trace2[0].output.starts_with("[信封修复]"),
        "{}",
        trace2[0].output
    );
    assert!(trace2[0].output.contains("换行"), "{}", trace2[0].output);
    assert!(
        trace2[0].args.contains("第一行\\n第二行"),
        "执行的是修好后的参数：{}",
        trace2[0].args
    );
    // 真实会话的形状：内容字符串写完、只差信封的收尾括号 → 补上就执行，不必让模型重发
    let note2 = s(&["demo", "m0", "note2.txt"]);
    let missing_brace = format!(
        "已读完，落盘。{{\"type\":\"tool\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"正文\"}}",
        note2
    );
    let mut m3 = member_with_tools(
        "m0",
        vec![
            missing_brace,
            "{\"type\":\"say\",\"text\":\"落好了\"}".to_string(),
        ],
        Arc::clone(&runner),
    );
    m3.tools.as_mut().expect("工具环境").repair = Arc::new(crate::adapters::UnambiguousRepair);
    let exec3 = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m3),
        "任务",
        &prompts,
        Default::default(),
    );
    let trace3 = exec3.traces.get("m0").expect("工具行");
    assert!(trace3[0].ok, "补上收尾括号后照常执行：{}", trace3[0].output);
    assert!(
        trace3[0].output.starts_with("[信封修复]") && trace3[0].output.contains("补上缺的收尾"),
        "要如实标注补了什么：{}",
        trace3[0].output
    );
    // 补不出来的（缺的是一个值而不是括号）不猜：照旧记失败行，并说清还差什么
    let hopeless = "{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":";
    let mut m4 = member_with_tools(
        "m0",
        vec![
            hopeless.to_string(),
            "{\"type\":\"say\",\"text\":\"知道了\"}".to_string(),
        ],
        Arc::clone(&runner),
    );
    m4.tools.as_mut().expect("工具环境").repair = Arc::new(crate::adapters::UnambiguousRepair);
    let exec4 = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m4),
        "任务",
        &prompts,
        Default::default(),
    );
    let trace4 = exec4.traces.get("m0").expect("工具行");
    assert!(
        !trace4[0].ok && trace4[0].output.contains("还差"),
        "{}",
        trace4[0].output
    );
}

#[test]
pub(crate) fn malformed_envelopes_are_classified_so_the_model_gets_the_right_fix() {
    // 类别是可判定的确切事实：模型据此能直接改对，而不是被笼统告知"JSON 不合法"。
    use crate::core::envelope::{parse, Malformed};
    // ① 字符串里直接换行（真实事故：write 的 content 里裸换行 → 整段 JSON 非法）
    let r = parse("{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\",\"content\":\"第一行\n第二行\"}}");
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
        Some(Malformed::Unclosed(crate::core::envelope::Tail {
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
    let texts = test_prompts().core.tool_texts;
    let control = texts.malformed_report(&Malformed::RawControl {
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
    let shape = texts.malformed_report(&Malformed::Shape("missing field name".to_string()));
    assert!(
        shape.contains("字段不合法") && shape.contains("missing field"),
        "{}",
        shape
    );
    let tab = texts.malformed_report(&Malformed::RawControl {
        ch: '\t',
        line: 1,
        tail: None,
    });
    // 只差收尾括号：要说清"还差 }"（而不是误导成"内容过长"）
    let brace = texts.malformed_report(&Malformed::Unclosed(crate::core::envelope::Tail {
        missing: "}".to_string(),
        in_string: false,
        envelopes: 1,
    }));
    assert!(brace.contains("还差 }"), "{}", brace);
    assert!(
        !brace.contains("分次写入"),
        "内容写完就别引导它去分次写：{}",
        brace
    );
    // 断在字符串中间才是"内容没写完"，这时才谈分次写
    let cut = texts.malformed_report(&Malformed::Unclosed(crate::core::envelope::Tail {
        missing: "}\"}".to_string(),
        in_string: true,
        envelopes: 1,
    }));
    // 一段回复里起了两段信封：要说清"只发一段"，而不是让它去补末尾括号（真实事故的形状）
    let multi = texts.malformed_report(&Malformed::Unclosed(crate::core::envelope::Tail {
        missing: "}}".to_string(),
        in_string: false,
        envelopes: 2,
    }));
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
    assert!(
        texts.malformed_unclosed_brace != texts.malformed_unclosed_string
            && texts.malformed_unclosed_string != texts.malformed_syntax
            && texts.malformed_syntax != texts.malformed_shape,
        "每一类的文案都要各说各的"
    );
}

#[test]
pub(crate) fn envelope_tool_parses_name_and_args() {
    let r = crate::core::envelope::parse(TOOL_CALL);
    assert_eq!(r.verb, crate::core::envelope::Verb::Tool);
    let inv = r.tools.first().cloned().expect("应有调用申请");
    assert_eq!(inv.name, "grep");
    assert!(
        inv.module.is_none(),
        "没写 module = None（单模块 agent 靠这个兜底）"
    );
    assert!(inv.args_json.contains("keyword"));
    // 带 module 的信封：trim 后非空才是 Some（空串按省略处理）。
    let with_mod = crate::core::envelope::parse(
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
    let blank_mod = crate::core::envelope::parse(
        "{\"type\":\"tool\",\"module\":\"  \",\"name\":\"read_txt\",\"args\":{}}",
    );
    assert!(blank_mod
        .tools
        .first()
        .cloned()
        .expect("应有调用申请")
        .module
        .is_none());
    // name 缺失 = 工具信封但不合法 → **独立的 malformed 信号**（不再按原文发言收录）。
    let bad = crate::core::envelope::parse("{\"type\":\"tool\",\"args\":{}}");
    assert_eq!(
        bad.verb,
        crate::core::envelope::Verb::Tool,
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
    let plain = crate::core::envelope::parse("没有信封的发言");
    assert!(plain.degraded && plain.tools.is_empty() && plain.text == "没有信封的发言");
    // 信封之外的正文才进 text（永不把信封 JSON 当文本）；只剩信封时 text 为空串。
    assert!(
        crate::core::envelope::parse(TOOL_CALL).text.is_empty(),
        "只剩信封 → text 空"
    );
    let prose = crate::core::envelope::parse(
        "先看一眼。{\"type\":\"tool\",\"name\":\"grep\",\"args\":{}}后记",
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
    use crate::core::session::stream_piece;
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
    let rows = transcript_rows(&with_live(|l| core.single_say(&sid, "跑一下", l)).unwrap());
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
pub(crate) fn prose_then_tool_envelope_keeps_prose_line_and_rebuilds_identically() {
    // 同一轮里「先写正文再发信封」：正文与思维链要落进文本行，且正文行不得含 JSON；
    // 重建上下文必须与实时历史逐条一致（工具轮的文本行不另推 assistant）。
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let n = s(&["w", "a", "n.txt"]);
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        format!("我先看看这个文件。{{\"type\":\"tool\",\"module\":\"a\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}}}", n),
        "{\"type\":\"say\",\"text\":\"写好了\"}".to_string(),
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
    let events = with_live(|l| core.single_say(&sid, "记一笔", l)).unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.len(),
        4,
        "用户行 / 正文行 / tool 行 / 答复行：{:?}",
        rows
    );
    assert!(
        rows[1].1.contains("我先看看这个文件。"),
        "正文要落进文本行：{:?}",
        rows[1]
    );
    assert!(!rows[1].1.contains('{'), "正文行不得含 JSON：{:?}", rows[1]);
    assert!(
        !rows[1].2 && rows[2].2 && !rows[3].2,
        "tool 行紧随正文行：{:?}",
        rows
    );
    assert!(
        !rows[2].1.contains('{'),
        "tool 行也不含 JSON：{:?}",
        rows[2]
    );
    assert_eq!(
        io.get(&["w", "a", "n.txt"]).as_deref(),
        Some("hi"),
        "工具真的跑了"
    );

    let live_h = core.single_history(&sid).unwrap();
    assert!(
        live_h.iter().any(|m| m.role == "assistant"
            && m.content.contains("我先看看这个文件。")
            && m.content.contains("\"type\":\"tool\"")),
        "这一轮的 assistant 消息 = assistant(raw)，正文与信封都在：{:?}",
        live_h
    );
    assert!(
        live_h
            .iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "{:?}",
        live_h
    );

    // 「重启」：同一份落盘历史交给新的 Core，重建后必须与实时历史逐条一致。
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2.rewind(&sid, 4).unwrap(); // 保留全部 4 行
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live_h),
        "重建上下文必须与实时历史逐条一致"
    );
}

#[test]
pub(crate) fn forced_final_tool_envelope_shows_no_json() {
    // 超限强制收尾后模型仍发信封：显示文本取信封之外的正文（没有正文就是空），JSON 绝不进转录。
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut script: Vec<String> = (0..MAX_TOOL_CALLS).map(|_| TOOL_CALL.to_string()).collect();
    script.push("到此为止。{\"type\":\"tool\",\"name\":\"grep\",\"args\":{}}".to_string());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut mod_a = module_of("a");
    mod_a
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "跑满", l)).unwrap();
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        MAX_TOOL_CALLS,
        "超限后不再执行工具"
    );
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        MAX_TOOL_CALLS,
        "{:?}",
        rows
    );
    let last = rows.last().expect("应有末行");
    assert!(
        last.1.contains("到此为止。"),
        "超限后的正文要如实收录：{:?}",
        last
    );
    assert!(
        !last.1.contains('{') && !last.1.contains("\"type\""),
        "显示文本不得出现 JSON：{:?}",
        last
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "整条转录都不得出现 JSON：{:?}",
        rows
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
    let system = crate::core::module::agent_system(
        &prompts,
        "m0",
        std::slice::from_ref(&mod_m0),
        "工具说明",
        crate::core::providers::ToolMode::Envelope,
    );
    assert!(
        system.contains("【模块工具参数】"),
        "系统提示要有参数段：{}",
        system
    );
    assert!(
        system.contains("- m0.grep") && system.contains("keyword（string，必填）"),
        "{}",
        system
    );

    let table = crate::core::engine::tool_table(std::slice::from_ref(&mod_m0));
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
    crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m2),
        "任务",
        &prompts,
        Default::default(),
    );
    let calls = runner2.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1, "合法调用照常执行");
    assert!(calls[0].2.contains("keyword"));

    // 没声明参数的工具照旧不校验（不给模块开发者添门槛）。
    let plain = module_of("m0");
    let plain_table = crate::core::engine::tool_table(std::slice::from_ref(&plain));
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
pub(crate) fn shipped_modules_scan_clean() {
    // 随仓模块（modules/）是产品内容的一部分：清单必须全部合法、id 与目录一致。
    let roster = crate::adapters::FsModules::new(PathBuf::from("modules")).scan();
    assert!(!roster.modules.is_empty(), "仓库应自带模块");
    assert!(
        roster.rejected.is_empty(),
        "随仓清单必须全部合法：{:?}",
        roster.rejected
    );
    // 随仓的三件就是演示工作流那三件：少了任何一件，演示就跑不起来。
    let ids: Vec<&str> = roster
        .modules
        .iter()
        .map(|m| m.manifest.id.as_str())
        .collect();
    for id in ["harvest", "indexer", "render"] {
        assert!(ids.contains(&id), "随仓模块少了 {}（现在是 {:?}）", id, ids);
    }
}

#[test]
pub(crate) fn tool_loop_cap_forces_final_answer() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "r".into(),
        ok: true,
    });
    let mut script: Vec<String> = (0..MAX_TOOL_CALLS).map(|_| TOOL_CALL.to_string()).collect();
    script.push("{\"type\":\"say\",\"text\":\"最终回报\"}".into());
    let mut m = member_with_tools("m0", script, Arc::clone(&runner));
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        MAX_TOOL_CALLS,
        "调用数封顶"
    );
    assert_eq!(
        exec.reports.get("m0").map(|s| s.as_str()),
        Some("最终回报"),
        "超限后强制收尾"
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

#[test]
pub(crate) fn sandbox_resolve_accepts_only_absolute_paths_inside_roots() {
    use crate::core::workspace::Place;
    let sb = test_sandbox("a1", &["data"]);
    // 绝对且在根内 → 通过（返回归一化后的绝对路径）
    let (place, path) = sb
        .resolve(&p(&["demo", "work", "notes", "a.txt"]))
        .expect("共享区可达");
    assert_eq!(place, Place::Shared);
    assert_eq!(path, abs(&["demo", "work", "notes", "a.txt"]));
    let (place, path) = sb
        .resolve(&p(&["demo", "a1", "b.txt"]))
        .expect("私有沙箱可达");
    assert_eq!(place, Place::Private);
    assert_eq!(path, abs(&["demo", "a1", "b.txt"]));
    let (place, path) = sb
        .resolve(&p(&["mods", "data", "c.txt"]))
        .expect("成员模块目录可达");
    assert_eq!(place, Place::Module("data".to_string()));
    assert_eq!(path, abs(&["mods", "data", "c.txt"]));
    // 允许的根就是这三处：错误文案要把它们列全
    let roots_line = s(&["demo", "work"]);
    // 拒绝：越界 / 非绝对路径（相对、裸文件名、带冒号前缀的伪路径）/ .. 与 . 段 / 空段 / 空
    let bad: Vec<String> = vec![
        p(&["outside", "x.txt"]),
        "a.txt".to_string(),
        "demo/work/a.txt".to_string(),
        "work:/a.txt".to_string(),
        "sandbox:/b.txt".to_string(),
        "module:data:/c.txt".to_string(),
        format!(
            "{}{}..{}x.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!(
            "{}{}a{}..{}b.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!("{}//a.txt", p(&["demo", "work"])),
        "   ".to_string(),
    ];
    for b in &bad {
        let err = sb.resolve(b).unwrap_err();
        assert!(
            err.contains(&roots_line),
            "错误要把允许的真实根列全（{}）：{}",
            b,
            err
        );
    }
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
pub(crate) fn agent_system_carries_the_real_roots() {
    // 提示词里给出的根必须是真实绝对路径（模型据此拼路径；外部工具也认它）。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let h = core.single_history(&sid).unwrap();
    let system = &h[0].content;
    assert!(
        system.contains(&s(&["w", "work"])),
        "system 要含共享区真实根：{}",
        system
    );
    assert!(
        system.contains(&s(&["w", "a"])),
        "system 要含私有沙箱真实根：{}",
        system
    );
    assert!(
        system.contains(&format!("：{}", s(&["a"]))),
        "system 要含模块目录真实根：{}",
        system
    );
    // 示例必须用真实根拼
    assert!(
        system.contains(&format!("\"path\":\"{}/note.txt\"", s(&["w", "work"]))),
        "示例要用真实根拼路径：{}",
        system
    );
    // 护栏：提示词里不得出现任何解析不了的路径写法（模型会照抄它）
    assert!(
        !system.contains("work:/") && !system.contains("sandbox:/"),
        "提示词只该给真实根：{}",
        system
    );
}

#[test]
pub(crate) fn sandboxes_lookup_is_by_agent() {
    let boxes = crate::core::workspace::Sandboxes {
        shared: abs(&["w", "work"]),
        list: vec![test_sandbox("甲", &["a"])],
    };
    assert!(boxes.for_agent("甲").is_some(), "按 agent 实例名取沙箱");
    assert!(boxes.for_agent("a").is_none(), "沙箱不再按模块 id 反查");
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
        out.output.contains(crate::core::systool::MODULE_WRITE_MARK),
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
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    let note = s(&["demo", "work", "note.txt"]);
    io.seed(
        &["demo", "work", "note.txt"],
        "第一段\n要改的句子\n第三段\n",
    );
    let mut obs = crate::core::systool::Observations::default();
    let edit = |obs: &mut crate::core::systool::Observations, args: &str| {
        let full = format!("{{\"path\":\"{}\",{}}}", note, args);
        crate::core::systool::execute(&sb, &io, obs, "edit", &full)
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
        (InMemorySysIo::new().marked(false, true), "超过单次读取上限"),
        (InMemorySysIo::new().marked(true, false), "非法 UTF-8"),
    ] {
        io.seed(&["demo", "work", "note.txt"], "abc");
        let mut obs = crate::core::systool::Observations::default();
        let out = crate::core::systool::execute(&sb, &io, &mut obs, "edit", &args);
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
    let io = InMemorySysIo::new();
    let sb = test_sandbox("a1", &[]);
    let note = s(&["demo", "work", "note.txt"]);
    let run = |obs: &mut crate::core::systool::Observations, tool: &str, args: String| {
        crate::core::systool::execute(&sb, &io, obs, tool, &args)
    };
    let mut obs = crate::core::systool::Observations::default();
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
    let mut obs2 = crate::core::systool::Observations::default();
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
    let mut obs3 = crate::core::systool::Observations::default();
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

/// 被供应商按长度截断的通道替身：正文照发，但 finish_reason = length。
pub(crate) struct TruncChat {
    pub(crate) script: Vec<String>,
}

impl Chat for TruncChat {
    fn complete(
        &mut self,
        _m: &[Msg],
        _opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        let text = if self.script.len() > 1 {
            self.script.remove(0)
        } else {
            self.script.first().cloned().unwrap_or_default()
        };
        Completion {
            raw: text,
            finish: "length".to_string(),
            calls: Vec::new(),
            error: None,
        }
    }
}

/// 一律回"被截断"的网关（验核心能不能把截断与写错分开）。
pub(crate) struct TruncGateway {
    pub(crate) script: Vec<String>,
}

impl ChatGateway for TruncGateway {
    fn probe_tools(&self, _c: &Channel) -> Result<crate::core::ports::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        (
            Box::new(TruncChat {
                script: self.script.clone(),
            }),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (
            Box::new(TruncChat {
                script: self.script.clone(),
            }),
            false,
        )
    }
}

/// 被用户中止的通道：回调一律返回 false（调用方要求停止），但仍返回一段"只差一个括号"的信封。
pub(crate) struct AbortChat {
    pub(crate) raw: String,
}

impl Chat for AbortChat {
    fn complete(
        &mut self,
        _m: &[Msg],
        _opts: CompleteOpts<'_>,
        on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        let _ = on(Chunk::Start);
        Completion::text(self.raw.clone())
    }
}

pub(crate) struct AbortGateway {
    pub(crate) raw: String,
}

impl ChatGateway for AbortGateway {
    fn probe_tools(&self, _c: &Channel) -> Result<crate::core::ports::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        (
            Box::new(AbortChat {
                raw: self.raw.clone(),
            }),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (
            Box::new(AbortChat {
                raw: self.raw.clone(),
            }),
            false,
        )
    }
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
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "写", l)).unwrap();
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
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "写", l)).unwrap();
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
    let sid2 = core2
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events2 = with_live(|l| core2.single_say(&sid2, "说", l)).unwrap();
    let rows = transcript_rows(&events2);
    assert!(
        rows.iter()
            .any(|r| r.1.contains("半句话") && r.1.contains("（本段被输出长度截断）")),
        "{:?}",
        rows
    );
}

/// 固定探测结论的网关：专测"结论怎么落到登记处"这一层策略（事实本身由适配器测）。
pub(crate) struct ProbeGateway {
    pub(crate) outcome: Arc<Mutex<crate::core::providers::ProbeOutcome>>,
}

impl ChatGateway for ProbeGateway {
    fn probe_tools(&self, _c: &Channel) -> Result<crate::core::ports::ProbeOutcome, String> {
        Ok(self.outcome.lock().expect("锁").clone())
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        (
            scripted(vec!["{\"type\":\"say\",\"text\":\"收到\"}".to_string()]),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (scripted(vec!["[]".to_string()]), true)
    }
}

#[test]
pub(crate) fn a_probe_writes_back_only_conclusive_results() {
    use crate::core::providers::{ProbeOutcome, ToolMode};
    let fresh = |outcome: ProbeOutcome| {
        let mut core = core_with_gateway(
            vec![],
            ProbeGateway {
                outcome: Arc::new(Mutex::new(outcome)),
            },
        );
        core.provider_upsert("p", "http://x", "k")
            .expect("登记供应商");
        core.model_upsert("m", "M", "api-m", "p", "")
            .expect("登记模型");
        core
    };
    let mode_of = |core: &Core| {
        core.model_views()
            .iter()
            .find(|v| v.id == "m")
            .map(|v| v.tools)
    };

    // 支持 → 写回 native
    let mut core = fresh(ProbeOutcome::Supported {
        detail: "真的调了".to_string(),
    });
    assert_eq!(
        mode_of(&core),
        Some(ToolMode::Envelope),
        "探测前是缺省 envelope"
    );
    assert!(matches!(
        core.probe_model_tools("m"),
        Ok(ProbeOutcome::Supported { .. })
    ));
    assert_eq!(mode_of(&core), Some(ToolMode::Native), "支持就写回 native");

    // 明确不支持 → 写回 envelope
    let mut core = fresh(ProbeOutcome::Unsupported {
        detail: "供应商说 tools 不认识".to_string(),
    });
    assert!(matches!(
        core.probe_model_tools("m"),
        Ok(ProbeOutcome::Unsupported { .. })
    ));
    assert_eq!(
        mode_of(&core),
        Some(ToolMode::Envelope),
        "不支持就老实回到信封"
    );

    // 无法判定 → 不改（不替用户拍板），但事实照样报回去
    let mut core = fresh(ProbeOutcome::Unknown {
        detail: "没发起调用".to_string(),
    });
    assert!(matches!(
        core.probe_model_tools("m"),
        Ok(ProbeOutcome::Unknown { .. })
    ));
    assert_eq!(
        mode_of(&core),
        Some(ToolMode::Envelope),
        "没法定论就不动登记处"
    );

    // 无此模型 → 如实报错
    assert!(core.probe_model_tools("ghost").is_err());
}

/// 原生通道的脚本替身：一步 = 一次"原生工具调用"或一段文本；
/// 同时记录每次请求带过来的工具声明与消息（用来断言"声明真的发出去了、结果真的回填了"）。
pub(crate) enum NativeStep {
    Calls(Vec<crate::core::ports::ToolCall>),
    Text(String),
}

/// 每次请求声明的工具（名字 + 参数 Schema）。
pub(crate) type DeclLog = Arc<Mutex<Vec<Vec<(String, serde_json::Value)>>>>;

/// 每次请求看到的消息。
pub(crate) type SeenLog = Arc<Mutex<Vec<Vec<Msg>>>>;

pub(crate) struct NativeChat {
    pub(crate) steps: Vec<NativeStep>,
    pub(crate) declared: DeclLog,
    pub(crate) seen: SeenLog,
}

impl Chat for NativeChat {
    fn complete(
        &mut self,
        messages: &[Msg],
        opts: CompleteOpts<'_>,
        _on: &mut dyn FnMut(Chunk) -> bool,
    ) -> Completion {
        self.seen.lock().expect("锁").push(messages.to_vec());
        let decls: Vec<(String, serde_json::Value)> = opts
            .tools
            .map(|ts| {
                ts.iter()
                    .map(|d| (d.name.clone(), d.parameters.clone()))
                    .collect()
            })
            .unwrap_or_default();
        self.declared.lock().expect("锁").push(decls);
        if self.steps.is_empty() {
            return Completion::text("{\"type\":\"say\",\"text\":\"脚本用完了\"}");
        }
        match self.steps.remove(0) {
            NativeStep::Calls(calls) => Completion {
                raw: String::new(),
                finish: "tool_calls".to_string(),
                calls,
                error: None,
            },
            NativeStep::Text(t) => Completion::text(t),
        }
    }
}

/// 原生形态的成员：沙箱用测试根，模块表为空，工具执行走内存 IO。
pub(crate) fn native_member(
    id: &str,
    io: Arc<InMemorySysIo>,
    steps: Vec<NativeStep>,
    declared: DeclLog,
    seen: SeenLog,
) -> Member {
    let chat: BoxedChat = Box::new(NativeChat {
        steps,
        declared,
        seen,
    });
    let mut m = Member::new(id, "职责".to_string(), chat);
    let mut modules = BTreeMap::new();
    modules.insert(
        "m0".to_string(),
        ModuleTools {
            root: abs(&["mods", "root"]),
            commands: BTreeMap::new(),
            books: BTreeMap::new(),
            parallel: BTreeSet::new(),
        },
    );
    let sb = test_sandbox(id, &[]);
    m.tools = Some(MemberTools {
        mode: crate::core::providers::ToolMode::Native,
        modules,
        observations: crate::core::systool::Observations::default(),
        repair: Arc::new(NoRepair),
        log: Arc::new(crate::core::ports::NoopLog),
        runner: Arc::new(SilentRunner),
        sandbox: sb.clone(),
        io,
        unavailable: BTreeMap::new(),
        fence: crate::core::fence::FenceSpec::from_sandbox(&sb, false),
        reply_seq: 0,
        llm: Default::default(),
    });
    m
}

#[test]
pub(crate) fn native_mode_declares_tools_and_runs_multiple_structured_calls() {
    use crate::core::ports::ToolCall;
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
    let trace = exec.traces.get("a").expect("应记一条失败的工具行");
    assert_eq!(trace.len(), 1);
    assert!(!trace[0].ok, "原生模式下信封不执行");
    assert_eq!(
        trace[0].output, prompts.core.tool_texts.native_no_envelope,
        "要如实说清本通道用原生调用"
    );
    assert_eq!(io.get(&["demo", "work", "out.md"]), None, "绝不落盘");
}

/// 声明可并发的读取**真的并发**，且结果一律按**原始调用顺序**回填。
/// 第一个文件故意慢：它会**后完成**，但结果仍必须排在前面（上下文里不许乱序）。
#[test]
pub(crate) fn declared_parallel_reads_overlap_and_results_keep_the_call_order() {
    use crate::core::ports::ToolCall;
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
    use crate::core::ports::ToolCall;
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
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
    use crate::core::ports::ToolCall;
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
        t.runner = runner;
        m
    }
    let prompts = test_prompts();
    // ① 声明 parallel：两个调用真的并发
    let runner = Arc::new(ParallelRunner::new(40));
    let mut m = grep_member(
        Arc::clone(&runner) as Arc<dyn ToolRunner + Send + Sync>,
        true,
    );
    let exec = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
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
    let _ = crate::core::engine::Execution::run(
        std::slice::from_mut(&mut m),
        "任务",
        &prompts,
        Default::default(),
    );
    assert_eq!(
        runner.peak_concurrent(),
        1,
        "未声明可并发 = 独占串行（峰值 {}）",
        runner.peak_concurrent()
    );
}

#[test]
pub(crate) fn changing_the_declared_mode_takes_effect_on_the_next_generation() {
    use crate::core::providers::ProbeOutcome;
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
    core.provider_upsert("p", "http://x", "k")
        .expect("登记供应商");
    core.model_upsert("m", "M", "api-m", "p", "")
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
    core.probe_model_tools("m").expect("探测");
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
    let sid = core
        .create_work(work("w", WorkMode::Single, &["m0"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "打补丁", l)).unwrap();
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
    let book = &prompts.core.builtin_tools;
    for name in crate::core::systool::names() {
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
        if crate::core::systool::is_freeform(name) {
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
    // 模型侧说明来自同一份声明
    let sb = test_sandbox("a1", &[]);
    let guide = crate::core::systool::guide(&prompts, &sb);
    assert!(guide.contains("【工具参数】"), "{}", guide);
    assert!(
        guide.contains("- offset（integer，缺省 1，不小于 1）"),
        "{}",
        guide
    );
    assert!(
        guide.contains("- ignore_case（boolean）：是否忽略大小写；省略即区分大小写"),
        "{}",
        guide
    );
    // patch 的写法说明也进系统提示（自由格式：正文不走 JSON）
    assert!(guide.contains("【改文件：用 patch"), "{}", guide);
    assert!(
        guide.contains("*** End File"),
        "每块要收尾这件事必须写清楚：{}",
        guide
    );
}

#[test]
pub(crate) fn core_collab_tool_modules_run_in_execution() {
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "  1 | 内容".into(),
        ok: true,
    });
    let mut member = BTreeMap::new();
    // 脚本排布：讨论开场 say → 讨论 step agree（收敛）→ 执行阶段 TOOL_CALL → 最终回报。
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"建议直接做\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
            TOOL_CALL.to_string(),
            "{\"type\":\"say\",\"text\":\"执行完毕，见依据\"}".to_string(),
        ],
    );
    let mut mod_a = module_of("a");
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), decl("python tools/grep.py"));
    mod_a.manifest.tools = manifest_tools;
    // 核心脚本：整理（方案）→ 验收（全过）。
    let mut core = core_with_runner(
        vec![mod_a],
        gw(
            member,
            vec![
                "{\"plan\":\"方案：查证后回报\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做 X\",\"objective\":\"把 X 做完\",\"assignee\":\"a\",\"deps\":[]}]}".into(),
                "[{\"item\":\"查证\",\"status\":\"pass\"}]".into(),
            ],
        ),
        Arc::clone(&runner),
    );
    let opened = core
        .create_work(collab_work("w", &["a"], false, "任务"))
        .unwrap();
    let sid = opened.sid;
    let mut events = opened.events;
    events.extend(core.collab_continue(&sid, CollabStep::Begin, "").unwrap());
    // 整理完停在**审查关卡**：点「同意」才继续（P4b 起协作的必经一步）。
    events.extend(
        core.collab_continue(&sid, CollabStep::ApprovePlan, "")
            .unwrap(),
    );
    // 工具只在执行阶段跑：发一条 tool 转录行 + 恰好一次进程调用。
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Transcript(ls)
            if ls.iter().any(|l| l.tool.is_some() && l.line.contains("[a:tool]") && l.line.contains("成功")))),
        "执行阶段的工具调用应发 tool 转录行",
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })),
        "验收应通过"
    );
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}
// ---------- 运行包：契约、包库、诊断、执行计划 ----------

#[test]
pub(crate) fn package_manifest_check_rejects_illegal_forms() {
    let check = crate::core::packages::check_manifest;
    assert!(
        check(&pkg("python", "3.12.4")).is_ok(),
        "prefix 类默认 kind"
    );
    assert!(
        check(&pkg_yaml("id: Python\nversion: 1\nprefix: opt/p")).is_err(),
        "id 只允许小写"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nkind: magic\nprefix: opt/p")).is_err(),
        "kind 只认 prefix / system"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1")).is_err(),
        "prefix 类必须给 prefix"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nprefix: opt/../etc")).is_err(),
        "前缀不能含 .."
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nprefix: opt\\\\rt")).is_err(),
        "前缀用 / 书写形式"
    );
    assert!(
        check(&pkg_yaml("id: cc\nversion: 1\nkind: system")).is_err(),
        "system 类必须给 provides_paths"
    );
    assert!(check(&pkg_yaml(
        "id: cc\nversion: 1\nkind: system\nprovides_paths: [usr/include]"
    ))
    .is_ok());
    assert!(
        check(&pkg_yaml("id: a\nversion: 1\nprefix: opt/a\nrequires: [a]")).is_err(),
        "requires 不能依赖自己"
    );
}

#[test]
pub(crate) fn module_runtimes_are_validated() {
    let mut m = module_of("a");
    m.manifest.runtimes = vec!["python".to_string(), "cc".to_string()];
    assert!(crate::core::module::check_runtimes(&m.manifest).is_ok());
    m.manifest.runtimes = vec!["Python".to_string()];
    assert!(
        crate::core::module::check_runtimes(&m.manifest).is_err(),
        "大写不合法"
    );
    m.manifest.runtimes = vec!["python".to_string(), "python".to_string()];
    assert!(
        crate::core::module::check_runtimes(&m.manifest)
            .unwrap_err()
            .contains("重复"),
        "重复声明要拒收"
    );
}

#[test]
pub(crate) fn module_tools_may_not_take_builtin_names() {
    let mut m = module_of("a");
    m.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    assert!(
        crate::core::module::check_tools(&m.manifest).is_ok(),
        "普通工具名可用"
    );
    for name in ["read", "write", "search"] {
        m.manifest
            .tools
            .insert(name.to_string(), decl("python tools/x.py"));
        let why = crate::core::module::check_tools(&m.manifest).unwrap_err();
        assert!(why.contains("保留名"), "内置工具名要拒收：{}", why);
        m.manifest.tools.remove(name);
    }
}

#[test]
pub(crate) fn library_keeps_versions_and_rejects_duplicates() {
    let lib = Library::build(
        vec![
            pkg("python", "3.12.4"),
            pkg("python", "3.11.9"),
            pkg("python", "3.12.4"),
            pkg_yaml("id: bad\nversion: 1"),
        ],
        vec!["x：package.yaml 非法".to_string()],
    );
    let versions: Vec<&str> = lib
        .versions_of("python")
        .iter()
        .map(|p| p.version.as_str())
        .collect();
    assert_eq!(
        versions,
        vec!["3.11.9", "3.12.4"],
        "同 (id, version) 只收一份，版本升序"
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("只收先出现的那份")),
        "{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("prefix")),
        "非法清单要说明原因：{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("package.yaml 非法")),
        "适配层拒收原因也要留：{:?}",
        lib.rejected
    );
    let caps = lib.capability_versions();
    assert_eq!(caps.get("python").map(|v| v.len()), Some(2));
}

#[test]
pub(crate) fn package_conflicts_flag_overlapping_paths() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: a\nversion: 1\nkind: system\nprovides_paths: [usr/lib]"),
            pkg_yaml("id: b\nversion: 1\nkind: system\nprovides_paths: [usr/lib/x86_64]"),
            pkg("node", "20.11.1"),
        ],
        Vec::new(),
    );
    let refs: Vec<&PackageManifest> = lib.packages.iter().collect();
    let got = crate::core::packages::conflicts(&refs);
    assert_eq!(got.len(), 1, "只有那对写进同一处的包冲突：{:?}", got);
    assert_eq!(got[0].0, "usr/lib");
    assert!(
        got[0].1.contains("a@1") && got[0].2.contains("b@1"),
        "{:?}",
        got
    );
}

/// 虚拟机档选型：基础根 + 不联网（定版留空 = 让库自己决定；多版本时报歧义）。
pub(crate) fn vm_spec() -> ExecSpec {
    ExecSpec {
        tier: Tier::Vm,
        base: Some("base-linux".to_string()),
        pins: BTreeMap::new(),
        net: false,
    }
}

/// 界面上的"能不能选"与创建/编辑的拒绝走同一个函数，所以这里钉住的就是那两处共同的事实。
/// 现在的判据是**逐项清单**：缺哪几项、每项怎么补，都要能读出来。
#[test]
pub(crate) fn vm_tier_readiness_gates_creation_and_editing() {
    // 本机档：任何机器上都能承载（不装载运行包、不要 guest）。
    let host = exec::tier_readiness(&ExecSpec::default(), None);
    assert!(host.ready(), "本机档没有前置条件");
    assert!(host.requirements.is_empty(), "本机档不该有虚拟机前置清单");
    assert!(exec::tier_refusal(&ExecSpec::default(), None).is_none());

    // 虚拟机档：guest 本体尚未接入是**所有机器**共同缺的一项，所以现在谁都不能建。
    let ghost = ExecSpec {
        tier: Tier::Vm,
        base: Some("definitely-not-a-real-base-root".to_string()),
        ..Default::default()
    };
    let r = exec::tier_readiness(&ghost, None);
    assert!(!r.ready(), "前置不齐就不成立：{:?}", r);
    let unmet: Vec<&str> = r.unmet().iter().map(|x| x.id).collect();
    assert!(
        unmet.contains(&"guest"),
        "guest 未接入要如实列出：{:?}",
        unmet
    );
    assert!(
        unmet.contains(&"base"),
        "填错的基础根要如实列出：{:?}",
        unmet
    );
    for item in r.unmet() {
        assert!(
            !item.how.is_empty(),
            "每一项没满足都要给出怎么补：{:?}",
            item
        );
        assert!(!item.detail.is_empty(), "每一项都要有现状描述：{:?}", item);
    }
    let why = exec::tier_refusal(&ghost, None).expect("不成立就要给可读理由");
    assert!(why.contains("虚拟机档现在不可用"), "{}", why);

    // 基础根在场：base 这一项要认出来（其余项照旧按事实）。
    let dir = crate::tests::scratch("tier-readiness");
    let real = ExecSpec {
        tier: Tier::Vm,
        base: Some(dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let r2 = exec::tier_readiness(&real, None);
    let base_item = r2
        .requirements
        .iter()
        .find(|x| x.id == "base")
        .expect("清单里要有基础根这一项");
    assert!(base_item.met, "在场的基础根要认出来：{:?}", base_item);
    // 严格：guest 未接入时**任何机器**都不能建虚拟机档会话。
    assert!(!r2.ready(), "guest 未接入期间一律不可用");
    let _ = std::fs::remove_dir_all(&dir);
}

/// QEMU 检测：登记了就用登记的路径，没登记就看 PATH；产品不自带、不下载。
#[test]
pub(crate) fn vm_requirements_report_qemu_registration() {
    let base = crate::tests::scratch("vm-req-qemu");
    let base_str = base.to_string_lossy().into_owned();
    let spec = ExecSpec {
        tier: Tier::Vm,
        base: Some(base_str.clone()),
        ..Default::default()
    };

    // 登记了一个不存在的路径：必须报"找不到"，并给出怎么补。
    let r = exec::tier_readiness(&spec, Some("definitely-not-qemu.exe"));
    let qemu = r
        .requirements
        .iter()
        .find(|x| x.id == "qemu")
        .expect("清单里要有 QEMU 这一项");
    assert!(!qemu.met, "不存在的路径不算找到：{:?}", qemu);
    assert!(!qemu.how.is_empty(), "没找到就要给怎么补：{:?}", qemu);

    // 登记一个真实存在的文件：要认出来（QEMU 是不是真的不重要——这里只钉"登记生效"）。
    let fake = base.join("qemu-system-x86_64");
    std::fs::write(&fake, b"stub").unwrap();
    let r2 = exec::tier_readiness(&spec, Some(fake.to_string_lossy().as_ref()));
    let qemu2 = r2
        .requirements
        .iter()
        .find(|x| x.id == "qemu")
        .expect("清单里要有 QEMU 这一项");
    assert!(qemu2.met, "登记的路径在场就要认出来：{:?}", qemu2);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
pub(crate) fn exec_host_tier_ignores_packages() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let plan = exec::plan(&ExecSpec::default(), &modules, &Library::default())
        .expect("本机档不装载运行包，不会因缺包失败");
    assert_eq!(plan.tier, Tier::Host);
    assert!(plan.packages.is_empty());
    assert!(plan.base.is_none());
    assert!(!plan.net, "默认不放行出站网络");
    let summary = exec::plan_summary(&plan);
    assert!(
        summary.contains("本机") && summary.contains("不放行"),
        "{}",
        summary
    );
}

#[test]
pub(crate) fn exec_vm_tier_reports_missing_ambiguous_and_unavailable() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let empty = Library::default();
    let spec = vm_spec();
    assert_eq!(
        exec::vm_diagnoses(&modules, &empty, &spec),
        vec![Diagnosis::Missing {
            module: "a".to_string(),
            capability: "python".to_string()
        }]
    );
    let un = exec::unavailable(&spec, &modules, &empty);
    assert_eq!(
        un.get("a"),
        Some(&vec!["python".to_string()]),
        "虚拟机档缺包 = 该模块工具不可用"
    );
    assert!(
        exec::unavailable(&ExecSpec::default(), &modules, &empty).is_empty(),
        "本机档一律可用"
    );
    // 缺包不拦会话：计划里没有可装载的包，那一步的降级由 unavailable 收口。
    let partial = exec::plan(&spec, &modules, &empty).expect("缺包不该拦会话");
    assert!(partial.packages.is_empty());
    let said = exec::diagnose_text(&exec::vm_diagnoses(&modules, &empty, &spec));
    assert!(
        said.contains("runtimes/"),
        "缺包的说法要告诉用户把包放哪：{}",
        said
    );
    // 多版本且未定版 = 不替用户选
    let two = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")],
        Vec::new(),
    );
    assert!(exec::vm_diagnoses(&modules, &two, &spec)
        .iter()
        .any(|d| matches!(d, Diagnosis::Ambiguous { .. })));
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &bad)
        .iter()
        .any(|d| matches!(d, Diagnosis::UnknownPin { .. })));
    // 多版本未定版 = 选型不成立：派生计划如实拒绝（这是用户要解决的选型问题，不是"缺包"）
    let refused = exec::plan(&spec, &modules, &two).unwrap_err();
    assert!(
        exec::diagnose_text(&refused).contains("多个版本"),
        "{}",
        exec::diagnose_text(&refused)
    );
    // 定版之后可以成立
    let ok = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &ok).is_empty());
    assert_eq!(exec::plan(&ok, &modules, &two).unwrap().packages.len(), 1);
}

#[test]
pub(crate) fn exec_vm_plan_pins_versions_and_orders_prefix_before_system() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/include]\nrequires: [binutils]"),
            pkg_yaml("id: binutils\nversion: 2.42\nprefix: opt/rt/binutils2.42"),
            pkg("python", "3.12.4"),
        ],
        Vec::new(),
    );
    let modules = vec![module_with_runtimes("a", &["python", "cc"])];
    let plan = exec::plan(&vm_spec(), &modules, &lib).expect("虚拟机档可成立");
    let ids: Vec<&str> = plan.packages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["binutils", "python", "cc"],
        "先独立前缀，后写进系统路径的包；包的 requires 走闭包"
    );
    assert_eq!(plan.packages[0].version, "2.42", "计划里是定版后的精确版本");
    assert_eq!(
        plan.packages[0].prefix, "opt/rt/binutils2.42",
        "独立前缀随计划走（装配按它挂载）"
    );
    assert_eq!(plan.packages[2].kind, "system", "写进系统路径的包排在最后");
    assert_eq!(plan.base.as_deref(), Some("base-linux"));
    assert!(!plan.net, "默认不放行出站网络");
}

#[test]
pub(crate) fn diagnose_text_spells_out_every_reason() {
    let missing = exec::diagnose_text(&[Diagnosis::Missing {
        module: "a".to_string(),
        capability: "python".to_string(),
    }]);
    assert!(
        missing.contains("模块 a") && missing.contains("python") && missing.contains("runtimes/"),
        "{}",
        missing
    );
    let ambiguous = exec::diagnose_text(&[Diagnosis::Ambiguous {
        capability: "python".to_string(),
        versions: vec!["3.11.9".to_string(), "3.12.4".to_string()],
    }]);
    assert!(
        ambiguous.contains("多个版本") && ambiguous.contains("3.12.4"),
        "{}",
        ambiguous
    );
    let bad = exec::diagnose_text(&[Diagnosis::UnknownPin {
        capability: "python".to_string(),
        version: "9.9".to_string(),
    }]);
    assert!(bad.contains("定版 9.9"), "{}", bad);
    let clash = exec::diagnose_text(&[Diagnosis::Conflict {
        path: "usr/lib".to_string(),
        a: "a@1".to_string(),
        b: "b@1".to_string(),
    }]);
    assert!(
        clash.contains("usr/lib") && clash.contains("a@1") && clash.contains("b@1"),
        "{}",
        clash
    );
}

/// 虚拟机档的承载校验：前置条件不具备时，**创建与编辑都如实拒绝**（用户环境问题，不是选型问题）。
/// 与界面上的"能不能选"同源（`exec::tier_readiness`），两处不会各说各话。
#[test]
pub(crate) fn vm_tier_is_refused_when_the_machine_cannot_carry_it() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["[]".into()]);
    let mut core = Core::new(
        Arc::new(InMemorySettings::with_tier(Tier::Vm)),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gw(member.clone(), vec!["[]".into()])),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败");
    // 创建路径的档位来自设置（基础根留空）：成立与否随本机而定，这里钉的是**接线**——
    // 机器承载不了就必须拒绝，且什么都不留下。
    let default_vm = ExecSpec {
        tier: Tier::Vm,
        ..ExecSpec::default()
    };
    let opened = core.create_work(work("vm-default", WorkMode::Single, &["a"]));
    if exec::tier_readiness(&default_vm, None).ready() {
        opened.expect("本机能承载虚拟机档时不该拒绝");
    } else {
        let err = opened.expect_err("本机承载不了虚拟机档就不许建");
        assert!(err.contains("虚拟机档现在不可用"), "{}", err);
        assert!(!core.session_exists("vm-default"), "拒绝就该什么都不留下");
    }

    // 编辑路径：基础根由用户给定（这里给一个不存在的），所以这一条不随机器变——必须拒绝、档位保持原样。
    let mut core2 = Core::new(
        Arc::new(InMemorySettings::new()),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gw(member, vec!["[]".into()])),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败");
    let sid = core2
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let err = core2
        .edit_session(
            &sid,
            SessionEdit {
                agents: vec![ConfigAgent {
                    name: "a".to_string(),
                    modules: vec!["a".to_string()],
                    model: String::new(),
                }],
                tier: "vm".to_string(),
                base: Some("definitely-not-a-real-base-root".to_string()),
                pins: BTreeMap::new(),
                net: false,
            },
        )
        .expect_err("基础根不在场就不许改入虚拟机档");
    assert!(
        err.contains("虚拟机档现在不可用") && err.contains("基础根不在场"),
        "{}",
        err
    );
    let cfg = core2.session_config(&sid).unwrap();
    assert_eq!(cfg.tier, "host", "被拒后档位保持原样");
    // 界面读的是**已保存的**配置（用户没提交的 base 输入后端看不到），两个字段必须自洽：
    // 可用时没有理由、不可用时必须有理由——界面据此决定禁用与说明。
    assert_eq!(
        cfg.vm_available,
        cfg.vm_unavailable_reason.is_empty(),
        "能不能选与为什么不能选必须说同一件事"
    );
}

#[test]
pub(crate) fn runtime_report_is_tier_aware() {
    let cores = |pkgs: Arc<InMemoryPackages>, modules: Vec<Module>| {
        core_with_pkgs(
            modules,
            gw(BTreeMap::new(), vec!["[]".into()]),
            Arc::new(SilentRunner),
            Arc::new(FakeCatalog::new(vec!["m".to_string()])),
            Arc::new(InMemoryHistory::new()),
            Arc::new(InMemorySysIo::new()),
            pkgs,
        )
    };
    let core = cores(
        Arc::new(InMemoryPackages::empty()),
        vec![module_with_runtimes("a", &["python"])],
    );
    let host = core.runtime_report(Tier::Host);
    assert_eq!(host.tier, "host");
    assert_eq!(host.declared.get("a"), Some(&vec!["python".to_string()]));
    assert_eq!(
        host.missing.get("a"),
        Some(&vec!["python".to_string()]),
        "档位无关的事实照实报"
    );
    assert!(host.available.is_empty());
    assert!(host.diagnoses.is_empty(), "本机档不做虚拟机档诊断");
    assert_eq!(core.runtime_report(Tier::Vm).diagnoses.len(), 1);
    // 包库里有包 = 缺失消失、诊断清空
    let with_pkg = cores(
        Arc::new(InMemoryPackages::with(&[
            "id: python\nversion: 3.12.4\nprefix: opt/rt/python3.12",
        ])),
        vec![module_with_runtimes("a", &["python"])],
    );
    let r = with_pkg.runtime_report(Tier::Vm);
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert_eq!(r.available.get("python"), Some(&vec!["3.12.4".to_string()]));
    assert!(r.diagnoses.is_empty(), "{:?}", r.diagnoses);
}

#[test]
pub(crate) fn module_without_runtime_is_denied_with_reason() {
    // 虚拟机档 + 空包库：模块声明的运行包没装载 → 工具不落进程，回执如实说缺哪个能力。
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"改用内置工具\"}".into(),
        ],
    );
    let mut mod_a = module_with_runtimes("a", &["python"]);
    mod_a
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    // 设置里的默认档位是**创建**时的档位来源：这里用本机档建（虚拟机档现在一律不可选），
    // 建好之后再把这条件裁成"已存在的虚拟机档会话"。
    let hist = Arc::new(InMemoryHistory::new());
    let mut core = Core::new(
        Arc::new(InMemorySettings::with_tier(Tier::Host)),
        Arc::clone(&hist) as Arc<dyn HistoryStore + Send + Sync>,
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![mod_a])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gw(member, vec!["[]".into()])),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&runner) as Arc<dyn ToolRunner + Send + Sync>,
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败");
    // 虚拟机档现在一律不可选（guest 本体尚未接入），所以**创建**走本机档；
    // 建好之后把落盘档位改成 vm——这正是"档位承载检查"与"缺包不拦会话"两件事的交界：
    // 已存在的会话照常打开、按 vm 档判工具可用性。
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    hist.force_tier(&sid, Tier::Vm);
    // 编辑一次会把内存里的会话丢掉；下一次访问按落盘 meta（已是 vm 档）重建，
    // 于是"工具可用性按 vm 档判"这条路才真的被走到。
    let base_dir = crate::tests::scratch("module-without-runtime-base");
    let mut rebuild = edit_of(vec![("a", &["a"], "")]);
    rebuild.tier = "vm".to_string();
    rebuild.base = Some(base_dir.to_string_lossy().into_owned());
    core.edit_session(&sid, rebuild).unwrap();
    // 访问一次（前端打开会话就是这一步）把会话按新配置重建；single_say 只认内存里已建好的会话。
    with_live(|l| core.continue_flow(&sid, l)).unwrap();
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "缺运行包时不落进程"
    );
    let texts = test_prompts().core.tool_texts;
    let expect = texts.render(
        &texts.module_unavailable,
        &[
            ("module", "a".to_string()),
            ("capability", "python".to_string()),
        ],
    );
    let feedback = core
        .single_history(&sid)
        .unwrap()
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[工具结果] a.grep"))
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(feedback.contains(&expect), "回执要用册子文案：{}", feedback);
    let lines = tool_line_texts(&events);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("grep") && l.contains("失败")),
        "失败也要发 tool 行：{:?}",
        lines
    );
    // 同一个模块在本机档照旧执行（本机档不装载运行包）。
    let mut member2 = BTreeMap::new();
    member2.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"跑完了\"}".into(),
        ],
    );
    let mut mod_b = module_with_runtimes("a", &["python"]);
    mod_b
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner2 = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut core2 = core_with_runner(
        vec![mod_b],
        gw(member2, vec!["[]".into()]),
        Arc::clone(&runner2),
    );
    let sid2 = core2
        .create_work(work("w2", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core2.single_say(&sid2, "干活", l)).unwrap();
    assert_eq!(
        runner2.calls.lock().expect("锁").len(),
        1,
        "本机档不受包库影响"
    );
}

// ---------- 配置视图：读、改、冻结 ----------

/// 造一条 agent 名单记录。
pub(crate) fn agent_meta(name: &str, modules: &[&str], model: Option<&str>) -> AgentMeta {
    AgentMeta {
        name: name.to_string(),
        transient: false,
        modules: modules.iter().map(|s| s.to_string()).collect(),
        model: model.map(|s| s.to_string()),
    }
}

/// 直接把一份 meta 放进内存历史（模拟"重启后从盘上读回该会话"）。
pub(crate) fn seed_session(
    hist: &Arc<InMemoryHistory>,
    name: &str,
    mode: &str,
    agents: Vec<AgentMeta>,
    exec: ExecSpec,
) {
    hist.create(&SessionMeta {
        name: name.to_string(),
        mode: mode.to_string(),
        delegate: false,
        modules: agents.iter().flat_map(|a| a.modules.clone()).collect(),
        task: None,
        ts: 1,
        agents,
        exec,
        parent: None,
        node: None,
    })
    .unwrap();
}

/// 一次编辑提交（名字 / 模块 / 模型；档位与定版默认本机档）。
pub(crate) fn edit_of(agents: Vec<(&str, &[&str], &str)>) -> SessionEdit {
    SessionEdit {
        agents: agents
            .into_iter()
            .map(|(n, ms, m)| ConfigAgent {
                name: n.to_string(),
                modules: ms.iter().map(|s| s.to_string()).collect(),
                model: m.to_string(),
            })
            .collect(),
        tier: "host".to_string(),
        base: None,
        pins: BTreeMap::new(),
        net: false,
    }
}

#[test]
pub(crate) fn session_config_reports_tier_missing_and_runtimes_dir() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], Some("m"))],
        ExecSpec {
            tier: Tier::Vm,
            ..Default::default()
        },
    );
    let core = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let cfg = core.session_config("w").unwrap();
    assert_eq!(cfg.sid, "w");
    assert_eq!(cfg.mode, "single");
    assert!(!cfg.started, "没有内容的会话 = 还没开过");
    assert_eq!(cfg.tier, "vm");
    assert_eq!(cfg.agents[0].model, "m");
    assert_eq!(
        cfg.runtime.missing.get("a"),
        Some(&vec!["python".to_string()])
    );
    assert!(
        cfg.runtimes_dir.ends_with("/runtimes"),
        "{}",
        cfg.runtimes_dir
    );
    assert!(core.session_config("没有这个会话").is_err());
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
    let history = core.single_history(&sid).unwrap();
    assert!(!history.is_empty(), "重建后上下文还在");
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
    let mut core = Core::new(
        Arc::new(InMemorySettings::new()),
        Arc::clone(&hist) as Arc<dyn crate::core::ports::HistoryStore + Send + Sync>,
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::clone(&fence) as Arc<dyn crate::core::ports::FenceHost + Send + Sync>,
        Arc::new(gw(BTreeMap::new(), vec!["[]".into()])),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败");
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

/// 手写信封通道的**批量调用**：一封 calls 数组里的多个调用各成一条工具行，结果按原序回填，
/// 且与原生通道一样「重建出来必须与实时逐条一致」（手写信封不涉及 role=tool）。
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
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "读两个文件", l)).unwrap();
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
    core2
        .rewind(&sid, transcript_rows(&events).len() as u64)
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

/// 两种信封形态**互斥**：一封里既有 name 又有 calls = 字段不合法 → 记一条失败工具行、一个工具都不执行。
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
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "写", l)).unwrap();
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

// ---------- 原生多调用的回放一致性（实时 vs 重建） ----------

/// 原生形态的网关：每次要通道就弹出一份脚本（第一份给实时会话，第二份给重建）。
pub(crate) struct NativeGateway {
    scripts: Mutex<Vec<Vec<NativeStep>>>,
}

impl ChatGateway for NativeGateway {
    fn probe_tools(&self, _c: &Channel) -> Result<crate::core::ports::ProbeOutcome, String> {
        Ok(crate::core::ports::ProbeOutcome::Supported {
            detail: "替身".to_string(),
        })
    }
    fn member_channel(&self, _c: Option<&Channel>, _id: &str) -> (BoxedChat, Option<String>) {
        let steps = self.scripts.lock().expect("锁").pop().unwrap_or_default();
        (
            Box::new(NativeChat {
                steps,
                declared: Arc::new(Mutex::new(Vec::new())),
                seen: Arc::new(Mutex::new(Vec::new())),
            }),
            None,
        )
    }
    fn core_channel(&self, _c: Option<&Channel>) -> (BoxedChat, bool) {
        (Box::new(FakeChat::new(vec!["[]".to_string()])), false)
    }
}

/// 指定网关 + 指定落盘历史装配一个核心（重建用例要读同一份转录）。
pub(crate) fn native_core(
    gateway: NativeGateway,
    history: Arc<InMemoryHistory>,
    io: Arc<InMemorySysIo>,
) -> Core {
    Core::new(
        Arc::new(InMemorySettings::new()),
        history,
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        Arc::new(gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(SilentRunner),
        io,
        Arc::new(NoRepair),
        Box::new(TestPrompts::ok()),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

/// 一次回复里的**多个**原生调用：实时历史与重建历史必须逐条一致（含 tool_calls 与 tool_call_id）。
/// 这就是原先不一致的那条：实时只推第一条调用的回执、第二条起什么都不推，重建却每条都推。
#[test]
pub(crate) fn native_multi_call_rebuilds_identically_to_live() {
    use crate::core::ports::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "a.txt"], "A1\nA2\n");
    io.seed(&["w", "a", "b.txt"], "B1\nB2\n");
    let a = s(&["w", "a", "a.txt"]);
    let b = s(&["w", "a", "b.txt"]);
    let scripts = || {
        vec![
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
        ]
    };
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![scripts()]),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.probe_model_tools("m")
        .expect("探测（把这条通道判成原生）");
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "读两个文件", l)).unwrap();
    let live = core.single_history(&sid).unwrap();

    // 转录行：两条工具行同属一次回复（回复号 = 该回复第一条工具行的 id）。
    let views = tool_views(&events);
    assert_eq!(views.len(), 2, "两个调用两条工具行");
    assert_eq!(
        views[0].reply, views[1].reply,
        "同一回复的两条工具行必须同号"
    );
    assert_eq!(views[0].call_id, "c1", "工具行记下供应商给的调用 id");
    assert_eq!(views[1].call_id, "c2");

    // 「重启」：同一份落盘历史交给新核心，按转录重建上下文——必须与实时逐条一致。
    drop(core);
    let mut core2 = native_core(
        NativeGateway {
            scripts: Mutex::new(Vec::new()),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2.probe_model_tools("m").expect("探测");
    let rows = transcript_rows(&events).len() as u64;
    core2.rewind(&sid, rows).unwrap();
    let rebuilt = core2.single_history(&sid).unwrap();
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| {
                (
                    m.role.clone(),
                    m.content.clone(),
                    m.tool_calls.len(),
                    m.tool_call_id.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live),
        "重建上下文必须与实时历史逐条一致（含 tool_calls / tool_call_id）"
    );
    assert_eq!(
        live.iter().filter(|m| !m.tool_calls.is_empty()).count(),
        1,
        "一次回复只推一条助手消息"
    );
}

/// 回档**按回复原子**：截在一次回复中间时整条回复一起丢，绝不留下"孤儿工具结果"。
#[test]
pub(crate) fn rewind_never_splits_a_reply() {
    use crate::core::ports::ToolCall;
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
    core.probe_model_tools("m").expect("探测");
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
    assert_eq!(history.len(), 2, "历史 = system + 用户：{:?}", history);
    assert!(
        history.iter().all(|m| m.tool_call_id.is_empty()),
        "历史里不许出现没有对应助手消息的孤儿工具结果：{:?}",
        history
    );
}
