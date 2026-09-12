//! 核心测试：全内存装配（InMemoryStore + VecSource + ScriptGateway），不碰文件系统。
//! 测试里的组合根 = 内存适配器；core 的可测性正是端口化的直接收益。

use crate::adapters::fake_chat::FakeChat;
use crate::core::engine::{Discussion, Member, TurnOut, MAX_ROUNDS};
use crate::core::module::{Module, ModuleManifest};
use crate::core::ports::{BoxedChat, Chat, ChatGateway, ModuleSource, Msg, PromptSource, ProviderStore};
use crate::core::prompt::{render, Prompts};
use crate::core::providers::{Provider, Registry};
use crate::core::session::{DirectSession, OmniSession};
use crate::core::{driven, Core};
use std::collections::BTreeMap;
use std::path::PathBuf;

// ---------- 内存适配器（测试组合根） ----------

struct InMemoryStore {
    reg: std::cell::RefCell<Registry>,
}

impl InMemoryStore {
    fn new() -> InMemoryStore {
        InMemoryStore { reg: std::cell::RefCell::new(Registry::default()) }
    }
}

impl ProviderStore for InMemoryStore {
    fn load(&self) -> Result<Registry, String> {
        Ok(self.reg.borrow().clone())
    }
    fn save(&self, r: &Registry) -> Result<(), String> {
        *self.reg.borrow_mut() = r.clone();
        Ok(())
    }
}

struct VecSource(Vec<Module>);

impl ModuleSource for VecSource {
    fn scan(&self) -> crate::core::module::Roster {
        crate::core::module::Roster { modules: self.0.clone(), rejected: Vec::new() }
    }
}

fn module_of(id: &str) -> Module {
    Module {
        manifest: ModuleManifest {
            id: id.to_string(),
            brief: format!("{} 的简介", id),
            system: format!("你负责{}", id),
            tools: Vec::new(),
            model: Default::default(),
        },
        root: PathBuf::from(id),
        selected_provider: None,
    }
}

fn scripted(s: Vec<String>) -> BoxedChat {
    Box::new(FakeChat::new(s))
}

/// 共享脚本队列：多条核心响应按 complete 次序弹出（末条重复兜底）。
/// 与真实通道时序一致：建通道时不消费，调用时才消费。
#[derive(Clone)]
struct SharedScript {
    q: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl Chat for SharedScript {
    fn complete(&mut self, _messages: &[Msg]) -> String {
        let mut q = self.q.borrow_mut();
        if q.len() > 1 {
            q.remove(0)
        } else {
            q.first().cloned().unwrap_or_default()
        }
    }
}

/// 脚本网关：按模块 id 回放各自脚本；核心通道走独立脚本队列。
struct ScriptGateway {
    member: BTreeMap<String, Vec<String>>,
    core: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl ChatGateway for ScriptGateway {
    fn member_channel(&self, _p: Option<&Provider>, id: &str) -> (BoxedChat, Option<String>) {
        let script = self.member.get(id).cloned().unwrap_or_else(|| {
            vec!["{\"type\":\"say\",\"text\":\"（演示）收到。\"}".to_string()]
        });
        (scripted(script), None)
    }
    fn core_channel(&self, _p: Option<&Provider>) -> (BoxedChat, bool) {
        (
            Box::new(SharedScript { q: std::rc::Rc::clone(&self.core) }),
            false,
        )
    }
}

struct TestPrompts;
impl PromptSource for TestPrompts {
    fn load(&self) -> Result<Prompts, String> {
        Ok(test_prompts())
    }
}

fn test_prompts() -> Prompts {
    serde_yaml::from_str::<Prompts>(include_str!("../prompts.yaml")).expect("内置提示词册必须合法")
}

fn core_with(modules: Vec<Module>, gateway: ScriptGateway) -> Core {
    Core::new(
        Box::new(InMemoryStore::new()),
        Box::new(VecSource(modules)),
        Box::new(gateway),
        Box::new(TestPrompts),
    )
    .expect("内存装配不应失败")
}

fn gw(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
    ScriptGateway { member, core: std::rc::Rc::new(std::cell::RefCell::new(core)) }
}

// ---------- 信封 ----------

#[test]
fn envelope_parse_clean() {
    let r = crate::core::envelope::parse("{\"type\":\"ask\",\"text\":\"要 A 还是 B？\"}");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Ask));
    assert_eq!(r.text, "要 A 还是 B？");
    assert!(!r.degraded);
}

#[test]
fn envelope_degraded_keeps_raw() {
    let r = crate::core::envelope::parse("这不是 JSON");
    assert!(r.degraded);
    assert_eq!(r.text, "这不是 JSON");
}

#[test]
fn envelope_wrapped_json_still_parses() {
    let r = crate::core::envelope::parse("好的：{\"type\":\"agree\",\"text\":\"同意\"} 以上。");
    assert!(matches!(r.verb, crate::core::envelope::Verb::Agree));
    assert!(!r.degraded);
}

// ---------- 提示词渲染层 ----------

#[test]
fn prompt_render_replaces_and_rejects_missing() {
    let ok = render("你好 {{name}}！", &[("name", "世界".to_string())]).unwrap();
    assert_eq!(ok, "你好 世界！");
    assert!(render("{{missing}}", &[]).is_err());
}

#[test]
fn prompt_book_loads_from_yaml() {
    let p = test_prompts();
    assert!(p.core.chat_protocol.contains("ask"));
    assert!(p.core.discuss.opener.contains("{{protocol}}"));
}

#[test]
fn prompt_render_keeps_single_braces() {
    let ok = render("输出 {\"a\":1} 和 {{x}}", &[("x", "Y".to_string())]).unwrap();
    assert_eq!(ok, "输出 {\"a\":1} 和 Y");
}

// ---------- 登记处解析链与密钥治理 ----------

#[test]
fn provider_resolution_chain() {
    let mut reg = Registry::default();
    reg.providers.insert("b".into(), Provider {
        kind: "llm".into(), base_url: "u".into(), api_key: "k".into(), models: vec!["m".into()],
    });
    reg.default = Some("b".into());
    assert!(reg.resolve(Some("a"), None).is_none());
    assert_eq!(reg.resolve(None, None).unwrap().0, "b");
}

#[test]
fn provider_lifecycle_and_key_never_leaks_to_view() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    core.provider_upsert("p1", "http://x", "sk-密钥XYZ", &["m".into()]).unwrap();
    assert_eq!(core.provider_default().as_deref(), Some("p1"));
    for line in core.provider_list() {
        assert!(!line.contains("sk-密钥XYZ"), "视图出现密钥：{}", line);
    }
    assert!(core.provider_remove("p1").unwrap());
    assert!(core.provider_default().is_none());
}

// ---------- 模块清单（内存来源） ----------

#[test]
fn roster_lists_modules() {
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec!["[]".into()]));
    let r = core.scan();
    let ids: Vec<_> = r.modules.iter().map(|m| m.manifest.id.clone()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}

// ---------- 讨论引擎 ----------

fn scripted_discussion(scripts: Vec<Vec<String>>, allow: bool) -> Discussion {
    let members: Vec<Member> = scripts
        .into_iter()
        .enumerate()
        .map(|(i, s)| Member::new(&format!("m{}", i), format!("职责{}", i), scripted(s)))
        .collect();
    Discussion::new(members, allow, test_prompts())
}

#[test]
fn discussion_full_agreement() {
    let mut d = scripted_discussion(
        vec![
            vec!["{\"type\":\"say\",\"text\":\"好\"}".into(), "{\"type\":\"agree\",\"text\":\"同意\"}".into()],
            vec!["{\"type\":\"say\",\"text\":\"行\"}".into(), "{\"type\":\"agree\",\"text\":\"同意\"}".into()],
        ],
        false,
    );
    d.open("任务");
    loop {
        match d.step() {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.contains("[m0:agree]")));
}

#[test]
fn discussion_ask_pauses() {
    let mut d = scripted_discussion(vec![vec!["{\"type\":\"ask\",\"text\":\"需要参数?\"}".into()]], false);
    d.open("任务");
    match d.step() {
        TurnOut::AskUser { member, question } => {
            assert_eq!(member, "m0");
            assert_eq!(question, "需要参数?");
        }
        _ => panic!("应暂停请教"),
    }
}

#[test]
fn discussion_leave_shrinks() {
    let mut d = scripted_discussion(vec![vec!["{\"type\":\"leave\",\"text\":\"撤了\"}".into()]], false);
    d.open("任务");
    let _ = d.step();
    assert!(d.members.iter().all(|m| !m.present));
}

#[test]
fn discussion_autonomy_archives_ask() {
    let mut d = scripted_discussion(
        vec![vec![
            "{\"type\":\"say\",\"text\":\"开场\"}".into(),
            "{\"type\":\"ask\",\"text\":\"细节?\"}".into(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".into(),
        ]],
        true,
    );
    d.open("任务");
    loop {
        match d.step() {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("自裁模式不该暂停"),
        }
    }
    assert!(d.transcript.iter().any(|l| l.contains("自裁")));
}

#[test]
fn discussion_round_cap_enforced() {
    let mut d = scripted_discussion(vec![vec!["{\"type\":\"say\",\"text\":\"继续\"}".into()]; 2], false);
    d.open("任务");
    loop {
        match d.step() {
            TurnOut::Round => continue,
            TurnOut::Done => break,
            TurnOut::AskUser { .. } => panic!("不该请教"),
        }
    }
    assert!(d.round > MAX_ROUNDS);
}

// ---------- 执行/验收 ----------

#[test]
fn execution_review_pass_and_fail_paths() {
    let prompts = test_prompts();
    let mut members = vec![Member::new("m0", "职责".to_string(), scripted(vec!["{\"type\":\"say\",\"text\":\"汇报内容\"}".into()]))];
    let mut exec = crate::core::engine::Execution::run(members.as_mut_slice(), "任务A", &prompts);
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("汇报内容"));
    let mut core_chat = scripted(vec!["[{\"item\":\"A\",\"status\":\"fail\",\"reason\":\"没做完\"}]".into()]);
    exec.review(core_chat.as_mut(), "方案", &prompts);
    assert!(!exec.all_pass());
    exec.rerun(members.as_mut_slice(), "任务A", "- A：没做完", &prompts);
    let mut core_chat2 = scripted(vec!["[{\"item\":\"A\",\"status\":\"pass\"}]".into()]);
    exec.review(core_chat2.as_mut(), "方案", &prompts);
    assert!(exec.all_pass());
}

#[test]
fn review_parse_failure_is_conservative_fail() {
    let prompts = test_prompts();
    let mut members = vec![Member::new("m0", "职责".to_string(), scripted(vec!["{\"type\":\"say\",\"text\":\"x\"}".into()]))];
    let mut exec = crate::core::engine::Execution::run(members.as_mut_slice(), "任务", &prompts);
    let mut core_chat = scripted(vec!["完全不是清单".to_string()]);
    exec.review(core_chat.as_mut(), "方案", &prompts);
    assert!(exec.items.is_empty());
    assert!(!exec.all_pass(), "解析失败必须保守判否");
}

// ---------- Core 门面（内存组合根） ----------

#[test]
fn core_direct_seeds_system_prompt() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["{\"type\":\"say\",\"text\":\"你好\"}".to_string()]);
    let core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let mut s: DirectSession = core.start_direct("a").unwrap();
    // 回归：直连历史首条必须是职责提示词（system），曾经丢失过。
    let h = s.history();
    assert_eq!(h[0].role, "system");
    assert!(h[0].content.contains("你负责a"));
    let reply = s.say("在吗");
    match reply {
        crate::core::SessionEvent::Transcript(lines) => assert!(lines[0].contains("[a]")),
        _ => panic!("应为转录事件"),
    }
}

#[test]
fn core_omni_merges_system_blocks() {
    let core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec!["[]".into()]));
    let s: OmniSession = core.start_omni("").unwrap();
    // 回归：全能首条 system 必须并入全部模块职责，且经册子渲染。
    let h = s.history();
    assert_eq!(h[0].role, "system");
    assert!(h[0].content.contains("你负责a") && h[0].content.contains("你负责b"));
}

#[test]
fn core_collab_demo_runs_full_five_stages() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意方案\"}".to_string(),
    ]);
    // 核心通道脚本：整理方案 → 验收清单（pass）。
    let core = core_with(vec![module_of("a")], gw(member, vec![
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]".to_string(),
    ]));
    let mut s = core.start_collab("a").unwrap();
    let _ = driven::set_task(&core, &mut s, "做个东西");
    assert!(matches!(s.pending, Some(crate::core::Pending::ConfirmBegin)));
    let events = driven::begin(&core, &mut s, false);
    assert!(events.iter().any(|e| matches!(e, crate::core::SessionEvent::Plan(_))));
    assert!(events.iter().any(|e| matches!(e, crate::core::SessionEvent::Delivery { ok: true, .. })));
}

#[test]
fn core_collab_delegated_slate_flow() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
    ]);
    let core = core_with(vec![module_of("a")], gw(member, vec![
        // 代拟 → 整理 → 验收。
        "{\"picks\":[{\"id\":\"a\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
    ]));
    let mut s = core.start_collab("?").unwrap();
    let ev = driven::set_task(&core, &mut s, "做个东西");
    assert!(matches!(s.pending, Some(crate::core::Pending::ConfirmSlate)));
    assert!(ev.iter().any(|e| matches!(e, crate::core::SessionEvent::Transcript(l) if l.iter().any(|x| x.contains("[代拟]")))));
    let _ = driven::confirm_slate(&core, &mut s, true);
    assert!(matches!(s.pending, Some(crate::core::Pending::ConfirmBegin)));
    let events = driven::begin(&core, &mut s, false);
    assert!(events.iter().any(|e| matches!(e, crate::core::SessionEvent::Delivery { ok: true, .. })));
}

#[test]
fn core_collab_slate_rejects_unknown_id() {
    let core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"picks\":[{\"id\":\"ghost\",\"why\":\"不存在\"},{\"id\":\"a\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案\"}".to_string(),
        "[{\"item\":\"x\",\"status\":\"pass\"}]".to_string(),
    ]));
    let mut s = core.start_collab("?").unwrap();
    let ev = driven::set_task(&core, &mut s, "任务");
    assert!(ev.iter().any(|e| matches!(e, crate::core::SessionEvent::Notice(n) if n.contains("ghost 不存在"))));
}

// ---------- 平衡提取器 ----------

#[test]
fn extract_balanced_array() {
    let s = "前缀 [ {\"a\":1}, {\"b\":\"}\"} ] 后缀";
    let got = crate::core::envelope::extract_json_array(s).unwrap();
    assert!(got.starts_with('[') && got.ends_with(']'));
    let obj = crate::core::envelope::extract_json_object("x {\"k\":\"{\"} y").unwrap();
    assert!(obj.starts_with('{') && obj.ends_with('}'));
}