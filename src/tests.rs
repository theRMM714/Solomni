//! 核心测试：全内存装配（InMemoryStore + VecSource + ScriptGateway），不碰文件系统。
//! 测试里的组合根 = 内存适配器；core 的可测性正是端口化的直接收益。
//! Web 阶段适配：适配器以 Arc 注入；会话经中心 id 收发；SharedScript 对齐真实通道时序。

use crate::adapters::fake_chat::FakeChat;
use crate::core::engine::{Discussion, Member, MemberTools, TurnOut, MAX_ROUNDS, MAX_TOOL_CALLS};
use crate::core::module::{Module, ModuleManifest};
use crate::core::ports::{BoxedChat, Chat, ChatGateway, ModuleSource, Msg, PromptSource, ProviderStore, ToolOutcome, ToolRunner};
use crate::core::prompt::{render, Prompts};
use crate::core::providers::{Provider, Registry};
use crate::core::{CollabStep, Core, Pending, SessionEvent};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------- 内存适配器（测试组合根） ----------

struct InMemoryStore {
    reg: Mutex<Registry>,
}

impl InMemoryStore {
    fn new() -> InMemoryStore {
        InMemoryStore { reg: Mutex::new(Registry::default()) }
    }
}

impl ProviderStore for InMemoryStore {
    fn load(&self) -> Result<Registry, String> {
        Ok(self.reg.lock().expect("锁").clone())
    }
    fn save(&self, r: &Registry) -> Result<(), String> {
        *self.reg.lock().expect("锁") = r.clone();
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
            tools: BTreeMap::new(),
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
/// 与真实通道时序一致：建通道时不消费，调用时才消费。Arc 分身共享（网关与测试两侧）。
/// 共享脚本队列（Mutex 版：需跨线程 Send+Sync）。
struct SharedScript {
    q: Arc<Mutex<Vec<String>>>,
}

impl Chat for SharedScript {
    fn complete(&mut self, _messages: &[Msg]) -> String {
        let mut q = self.q.lock().expect("脚本队列锁");
        if q.len() > 1 {
            q.remove(0)
        } else {
            q.first().cloned().unwrap_or_default()
        }
    }
}

/// 脚本网关：按模块 id 回放各自脚本；核心通道走共享队列。
struct ScriptGateway {
    member: BTreeMap<String, Vec<String>>,
    core: Arc<Mutex<Vec<String>>>,
}

impl ChatGateway for ScriptGateway {
    fn member_channel(&self, _p: Option<&Provider>, id: &str) -> (BoxedChat, Option<String>) {
        let script = self.member.get(id).cloned().unwrap_or_else(|| {
            vec!["{\"type\":\"say\",\"text\":\"（演示）收到。\"}".to_string()]
        });
        (scripted(script), None)
    }
    fn core_channel(&self, _p: Option<&Provider>) -> (BoxedChat, bool) {
        (Box::new(SharedScript { q: Arc::clone(&self.core) }), false)
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
    core_with_runner(modules, gateway, Arc::new(SilentRunner))
}

fn core_with_runner(modules: Vec<Module>, gateway: ScriptGateway, runner: Arc<impl ToolRunner + Send + Sync + 'static>) -> Core {
    Core::new(
        Arc::new(InMemoryStore::new()),
        Arc::new(VecSource(modules)),
        Arc::new(gateway),
        runner,
        Box::new(TestPrompts),
        Arc::new(crate::core::ports::NoopLog),
    )
    .expect("内存装配不应失败")
}

fn gw(member: BTreeMap<String, Vec<String>>, core: Vec<String>) -> ScriptGateway {
    ScriptGateway { member, core: Arc::new(Mutex::new(core)) }
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
    for v in core.provider_views() {
        assert!(!format!("{:?}", v).contains("sk-密钥XYZ"));
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

// ---------- Core 门面：会话中心（内存组合根） ----------

#[test]
fn core_direct_seeds_system_prompt() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["{\"type\":\"say\",\"text\":\"你好\"}".to_string()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let (sid, _open) = core.start_direct("a").unwrap();
    // 回归：直连历史首条必须是职责提示词（system），曾经丢失过。
    let h = core.direct_history(sid).unwrap();
    assert_eq!(h[0].role, "system");
    assert!(h[0].content.contains("你负责a"));
    let events = core.direct_say(sid, "在吗").unwrap();
    match &events[0] {
        SessionEvent::Transcript(lines) => assert!(lines[0].contains("[a]")),
        _ => panic!("应为转录事件"),
    }
}

#[test]
fn core_omni_merges_system_blocks() {
    let mut core = core_with(vec![module_of("a"), module_of("b")], gw(BTreeMap::new(), vec!["[]".into()]));
    let (sid, _open) = core.start_omni("").unwrap();
    // 回归：全能首条 system 必须并入全部模块职责，且经册子渲染。
    let h = core.omni_history(sid).unwrap();
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
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\",\"evidence\":\"已做\"}]".to_string(),
    ]));
    let sid = core.start_collab("a").unwrap();
    let _events = core.collab_continue(sid, CollabStep::SetTask, "做个东西").unwrap();
    assert!(matches!(core.collab_pending(sid), Ok(Some(Pending::ConfirmBegin))));
    let events = core.collab_continue(sid, CollabStep::Begin, "yes").unwrap();
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Plan(_))));
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
    // 终结会话已被中心回收（再查询挂起状态应报无此会话）。
    assert!(core.collab_pending(sid).is_err());
}

#[test]
fn core_collab_delegated_slate_flow() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
    ]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec![
        // 代拟 → 整理 → 验收。
        "{\"picks\":[{\"id\":\"a\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案：A 做 X\"}".to_string(),
        "[{\"item\":\"做 X\",\"status\":\"pass\"}]".to_string(),
    ]));
    let sid = core.start_collab("?").unwrap();
    let ev = core.collab_continue(sid, CollabStep::SetTask, "做个东西").unwrap();
    assert!(matches!(core.collab_pending(sid), Ok(Some(Pending::ConfirmSlate))));
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Transcript(l) if l.iter().any(|x| x.contains("[代拟]")))));
    let _ = core.collab_continue(sid, CollabStep::ConfirmSlate, "yes").unwrap();
    assert!(matches!(core.collab_pending(sid), Ok(Some(Pending::ConfirmBegin))));
    let events = core.collab_continue(sid, CollabStep::Begin, "yes").unwrap();
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })));
}

#[test]
fn core_collab_slate_rejects_unknown_id() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec![
        "{\"picks\":[{\"id\":\"ghost\",\"why\":\"不存在\"},{\"id\":\"a\",\"why\":\"对口\"}]}".to_string(),
        "{\"type\":\"say\",\"text\":\"方案\"}".to_string(),
        "[{\"item\":\"x\",\"status\":\"pass\"}]".to_string(),
    ]));
    let sid = core.start_collab("?").unwrap();
    let ev = core.collab_continue(sid, CollabStep::SetTask, "任务").unwrap();
    assert!(ev.iter().any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("ghost 不存在"))));
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

// ---------- 工具执行器 ----------

/// 守护 runner：任何调用即失败（守护不该用工具的路径）。
struct SilentRunner;
impl ToolRunner for SilentRunner {
    fn run(&self, _root: &std::path::Path, _command: &str, _args: &str) -> ToolOutcome {
        panic!("不应调用工具");
    }
}

/// 记录型 runner：记录 (root, command, args)，回放固定输出。
struct RecordingRunner {
    calls: Mutex<Vec<(PathBuf, String, String)>>,
    out: String,
    ok: bool,
}
impl ToolRunner for RecordingRunner {
    fn run(&self, root: &std::path::Path, command: &str, args_json: &str) -> ToolOutcome {
        self.calls
            .lock()
            .expect("锁")
            .push((root.to_path_buf(), command.to_string(), args_json.to_string()));
        ToolOutcome { ok: self.ok, output: self.out.clone() }
    }
}

const TOOL_CALL: &str = "{\"type\":\"tool\",\"name\":\"grep\",\"args\":{\"keyword\":\"x\"}}";

/// 带工具环境的成员：声明 grep → python tools/grep.py。
fn member_with_tools(id: &str, script: Vec<String>, runner: Arc<impl ToolRunner + Send + Sync + 'static>) -> Member {
    let mut commands = BTreeMap::new();
    commands.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut m = Member::new(id, "职责".to_string(), scripted(script));
    m.tools = Some(MemberTools { root: PathBuf::from("mods/root"), commands, runner });
    m
}

#[test]
fn envelope_tool_parses_name_and_args() {
    let r = crate::core::envelope::parse(TOOL_CALL);
    assert_eq!(r.verb, crate::core::envelope::Verb::Tool);
    let inv = r.tool.expect("应有调用申请");
    assert_eq!(inv.name, "grep");
    assert!(inv.args_json.contains("keyword"));
    // name 缺失 = 不合法 tool 信封 → 不猜测，降级按原文收录。
    let bad = crate::core::envelope::parse("{\"type\":\"tool\",\"args\":{}}");
    assert!(bad.tool.is_none());
    assert!(bad.text.contains("tool"));
}

#[test]
fn tool_loop_runs_declared_tool() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "  3 | 命中行".into(), ok: true });
    let mut m = member_with_tools(
        "m0",
        vec![TOOL_CALL.into(), "{\"type\":\"say\",\"text\":\"完成\"}".into()],
        Arc::clone(&runner),
    );
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("完成"));
    let calls = runner.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1, "声明过的工具应恰好执行一次");
    assert_eq!(calls[0].0, PathBuf::from("mods/root"), "工具进程工作目录 = 模块工作区");
    assert_eq!(calls[0].1, "python tools/grep.py", "命令来自模块清单");
    assert!(calls[0].2.contains("keyword"), "参数以 JSON 原样送达");
    let trace = exec.traces.get("m0").expect("轨迹应入册");
    assert!(trace.iter().any(|t| t.contains("grep") && t.contains("成功")));
}

#[test]
fn tool_loop_rejects_undeclared_tool() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: String::new(), ok: true });
    let mut m = member_with_tools(
        "m0",
        vec!["{\"type\":\"tool\",\"name\":\"nope\",\"args\":{}}".into(), "{\"type\":\"say\",\"text\":\"完成\"}".into()],
        Arc::clone(&runner),
    );
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(std::slice::from_mut(&mut m), "任务", &prompts);
    assert!(runner.calls.lock().expect("锁").is_empty(), "未声明的工具绝不落进程");
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("完成"));
    let trace = exec.traces.get("m0").expect("轨迹应入册");
    assert!(trace.iter().any(|t| t.contains("未声明")));
}

#[test]
fn shipped_modules_scan_clean() {
    // 随仓模块（modules/）是产品内容的一部分：清单必须全部合法、id 与目录一致。
    let roster = crate::adapters::FsModules::new(PathBuf::from("modules")).scan();
    assert!(!roster.modules.is_empty(), "仓库应自带模块");
    assert!(roster.rejected.is_empty(), "随仓清单必须全部合法：{:?}", roster.rejected);
    assert!(roster.modules.iter().any(|m| m.manifest.id == "summarizer"));
    assert!(roster.modules.iter().any(|m| m.manifest.id == "reviewer"));
}

#[test]
fn tool_loop_cap_forces_final_answer() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "r".into(), ok: true });
    let mut script: Vec<String> = (0..MAX_TOOL_CALLS).map(|_| TOOL_CALL.to_string()).collect();
    script.push("{\"type\":\"say\",\"text\":\"最终回报\"}".into());
    let mut m = member_with_tools("m0", script, Arc::clone(&runner));
    let prompts = test_prompts();
    let exec = crate::core::engine::Execution::run(std::slice::from_mut(&mut m), "任务", &prompts);
    assert_eq!(runner.calls.lock().expect("锁").len(), MAX_TOOL_CALLS, "调用数封顶");
    assert_eq!(exec.reports.get("m0").map(|s| s.as_str()), Some("最终回报"), "超限后强制收尾");
}

#[test]
fn core_direct_tool_flow_injects_result_into_history() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "  3 | 依据行".into(), ok: true });
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        TOOL_CALL.to_string(),
        "{\"type\":\"say\",\"text\":\"依据第 3 行，结论成立\"}".to_string(),
    ]);
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    let mut mod_a = module_of("a");
    mod_a.manifest.tools = manifest_tools;
    let mut core = core_with_runner(vec![mod_a], gw(member, vec!["[]".into()]), Arc::clone(&runner));
    let (sid, _open) = core.start_direct("a").unwrap();
    let events = core.direct_say(sid, "核对一下").unwrap();
    // 工具轨迹以 Notice 呈现，最终答复入转录。
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("[a:工具]") && n.contains("成功"))));
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Transcript(lines) if lines[0].contains("结论成立"))));
    // 历史完整：用户消息 → 工具信封原文 → 工具结果 → 最终答复。
    let h = core.direct_history(sid).unwrap();
    assert!(h.iter().any(|m| m.role == "user" && m.content.contains("[工具结果] grep")), "工具结果必须回注上下文");
    assert!(h.iter().any(|m| m.role == "assistant" && m.content.contains("结论成立")));
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}

#[test]
fn core_collab_tool_modules_run_in_execution() {
    let runner = Arc::new(RecordingRunner { calls: Mutex::new(Vec::new()), out: "  1 | 内容".into(), ok: true });
    let mut member = BTreeMap::new();
    // 脚本排布：讨论开场 say → 讨论 step agree（收敛）→ 执行阶段 TOOL_CALL → 最终回报。
    member.insert("a".to_string(), vec![
        "{\"type\":\"say\",\"text\":\"建议直接做\"}".to_string(),
        "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        TOOL_CALL.to_string(),
        "{\"type\":\"say\",\"text\":\"执行完毕，见依据\"}".to_string(),
    ]);
    let mut mod_a = module_of("a");
    let mut manifest_tools = BTreeMap::new();
    manifest_tools.insert("grep".to_string(), "python tools/grep.py".to_string());
    mod_a.manifest.tools = manifest_tools;
    // 核心脚本：整理（方案）→ 验收（全过）。
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec![
            "{\"type\":\"say\",\"text\":\"方案：查证后回报\"}".into(),
            "[{\"item\":\"查证\",\"status\":\"pass\"}]".into(),
        ]),
        Arc::clone(&runner),
    );
    let sid = core.start_collab("a").unwrap();
    let mut events = core.collab_continue(sid, CollabStep::SetTask, "任务").unwrap();
    events.extend(core.collab_continue(sid, CollabStep::Begin, "").unwrap());
    // 工具只在执行阶段跑：轨迹呈现 + 恰好一次进程调用。
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Notice(n) if n.contains("[a:工具]") && n.contains("成功"))), "执行阶段工具轨迹应呈现");
    assert!(events.iter().any(|e| matches!(e, SessionEvent::Delivery { ok: true, .. })), "验收应通过");
    assert_eq!(runner.calls.lock().expect("锁").len(), 1);
}