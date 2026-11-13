//! 目的：终端渲染的用例与替身：启动页、花名册、菜单、事件词汇的逐行断言。
//! 管：造核心/模块/登记处替身与事件夹具，并用 `term::capture` 断言行文。
//! 不管：生产渲染实现、交互流程与命令解析。
//! 联动：`cli/view.rs`、`cli/term.rs`、`cli/theme.rs`、`docs/presentation/cli/unit-map.md`

use super::*;
use crate::capabilities::conductor::api::{
    AgentSuggestion, ConductorHandle, ConductorOps, RuntimeDiagnosis, RuntimeReport, TierChoices,
    WorkMode,
};
use crate::capabilities::llm::api::{ProbeOutcome, ReplayReport};
use crate::capabilities::modules::api::{ModulesOps, Roster};
use crate::capabilities::packages::api::Problem;
use crate::capabilities::registry::api::{AgentView, AppSettings, ModelDraft, RegistryOps};
use crate::capabilities::registry::domain::agents::Agent;
use crate::capabilities::session::api::{CheckView, ContextUsage, LineView, ToolCallView};
use crate::capabilities::session::domain::events::{
    DecisionAnswer, DecisionEnvelope, DecisionMessage,
};
use crate::capabilities::taskchain::api::{NodeStatus, TaskChain, TaskNode};
use crate::tests::doubles::{
    InMemorySettings, core_with_settings_modules, module_of, module_with_runtimes,
};
use crate::tests::ops_with;
use std::collections::BTreeMap;
use std::sync::Arc;

fn ui(tty: bool) {
    let level = if tty {
        theme::Level::True
    } else {
        theme::Level::None
    };
    term::test_ui(tty, 80, false, true, level);
}
fn said(f: impl FnOnce()) -> Vec<String> {
    term::capture(f).1
}
/// 目的：某一行包含片段即算命中（颜色转义在前，不硬编码整行）。
fn has(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|l| l.contains(needle))
}
fn tool(module: &str, ok: bool, args: &str) -> ToolCallView {
    ToolCallView {
        speaker: "甲".into(),
        module: module.into(),
        name: "ls".into(),
        ok,
        args: args.into(),
        output: String::new(),
        call_id: "c1".into(),
        reply: 1,
        read: None,
    }
}
fn check(item: &str, status: &str) -> CheckView {
    CheckView {
        item: item.into(),
        status: status.into(),
        note: format!("{item}注"),
    }
}
fn usage(exact: bool) -> ContextUsage {
    ContextUsage {
        prompt_tokens: 12_000,
        completion_tokens: 345,
        total_tokens: 12_345,
        window: 128_000,
        exact,
        cache_read: None,
        cache_creation: None,
    }
}
fn map(rows: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    rows.iter()
        .map(|(k, vs)| (k.to_string(), vs.iter().map(|v| v.to_string()).collect()))
        .collect()
}
/// 目的：运行报告：各分区都有内容；诊断一条缺包（应被过滤）一条多版本。
fn report() -> RuntimeReport {
    RuntimeReport {
        tier: "vm".into(),
        declared: map(&[("a", &["qemu"])]),
        available: map(&[("qemu", &["9.2", "9.1"])]),
        missing: map(&[("b", &["qemu"])]),
        diagnoses: vec![
            RuntimeDiagnosis::Package(Problem::Missing {
                module: "b".into(),
                capability: "qemu".into(),
            }),
            RuntimeDiagnosis::Package(Problem::Ambiguous {
                capability: "qemu".into(),
                versions: vec!["9.2".into(), "9.1".into()],
            }),
        ],
        tier_ready: false,
        tier_missing: Vec::new(),
        rejected: vec!["坏的清单".into()],
        rejected_packages: vec!["坏的包".into()],
    }
}
fn chain() -> TaskChain {
    let node = TaskNode {
        id: "n1".into(),
        title: "写测试".into(),
        objective: String::new(),
        assignee: "a".into(),
        deps: Vec::new(),
        status: NodeStatus::Pending,
        sub_session: None,
        report: None,
        acceptance: None,
        reported: false,
    };
    TaskChain { nodes: vec![node] }
}
fn line(kind: &str, speaker: &str, text: &str) -> LineView {
    LineView {
        kind: kind.into(),
        speaker: speaker.into(),
        line: text.into(),
        ..Default::default()
    }
}
fn tool_line(module: &str, ok: bool, args: &str) -> LineView {
    LineView {
        kind: "tool".into(),
        tool: Some(tool(module, ok, args)),
        ..Default::default()
    }
}
fn transcript() -> Vec<LineView> {
    let mut compacted = line("compacted", "", "旧内容");
    compacted.payload = Some(serde_json::json!({"up_to": 3}));
    vec![
        compacted,
        tool_line("a", true, "ls  -l"),
        tool_line("", false, ""),
        line("tool", "", ""),
        LineView::user("说", "你好".into()),
        line("", "用户", "自称用户"),
        line("round", "", "第 2 轮"),
        LineView::system("系统", "提醒".into()),
        line("system", "", "多行\n第二行"),
        line("", "", "裸行"),
        LineView::speech("甲", "说", "正文".into()),
    ]
}
fn vocabulary() -> Vec<SessionEvent> {
    vec![
        SessionEvent::Notice("通告".into()),
        SessionEvent::Transcript(transcript()),
        SessionEvent::DiscussionDone {
            round: 1,
            over_cap: false,
        },
        SessionEvent::Plan("第一步\n第二步".into()),
        SessionEvent::PlanReview {
            plan: "方案".into(),
            chain: chain(),
        },
        SessionEvent::NodeStarted {
            node: "n1".into(),
            sid: "s1".into(),
            assignee: "a".into(),
        },
        SessionEvent::Report {
            id: "r1".into(),
            text: "做完了".into(),
            rework: 0,
        },
        SessionEvent::Report {
            id: "r2".into(),
            text: "改好了".into(),
            rework: 1,
        },
        SessionEvent::Review {
            items: Vec::new(),
            raw: "原文".into(),
        },
        SessionEvent::Review {
            items: vec![
                check("清单", "pass"),
                check("第二条", "fail"),
                check("第三条", "待定"),
            ],
            raw: String::new(),
        },
        SessionEvent::Delivery {
            ok: true,
            over_rework: false,
        },
        SessionEvent::Delivery {
            ok: false,
            over_rework: true,
        },
        SessionEvent::Delivery {
            ok: false,
            over_rework: false,
        },
        SessionEvent::Ended,
        SessionEvent::DecisionCard {
            card: DecisionCard {
                id: "c1".into(),
                envelope: DecisionEnvelope {
                    role: "gate".into(),
                    name: "甲".into(),
                },
                message: DecisionMessage {
                    title: "选一个".into(),
                    body: String::new(),
                    detail: String::new(),
                },
                options: Vec::new(),
            },
            gate: "g".into(),
            payload: serde_json::Value::Null,
            waiting: Vec::new(),
        },
        SessionEvent::DecisionAnswer(DecisionAnswer {
            card: "c1".into(),
            by: "甲".into(),
            option: "o1".into(),
            note: String::new(),
        }),
        SessionEvent::DecisionVoid {
            cards: vec!["c1".into(), "c2".into()],
            reason: "会话停了".into(),
        },
        SessionEvent::Delta {
            speaker: "甲".into(),
            kind: "text".into(),
            text: "…".into(),
        },
        SessionEvent::Working {
            agent: Some("甲".into()),
        },
        SessionEvent::Working { agent: None },
        SessionEvent::ToolCall(tool("a", true, "ls")),
        SessionEvent::Rewound { events: Vec::new() },
        SessionEvent::Usage {
            usage: usage(true),
            line: 7,
        },
    ]
}
/// 目的：核心替身：运行报告由用例指定（覆盖报告各分区与报告失败）。
struct FakeCore(Result<RuntimeReport, String>);
impl ConductorOps for FakeCore {
    fn runtime_report(&self, _: Tier) -> Result<RuntimeReport, String> {
        self.0.clone()
    }
    fn tier_choices(&self) -> Result<TierChoices, String> {
        Err("没装档位选择".into())
    }
    fn suggest_models(&self, _: &str, _: WorkMode) -> Result<Vec<AgentSuggestion>, String> {
        Err("没装推荐".into())
    }
}
/// 目的：模块公地替身：清单读取失败。
struct BrokenModules;
impl ModulesOps for BrokenModules {
    fn roster(&self) -> Result<Roster, String> {
        Err("模块坏了".into())
    }
    fn module_enabled(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
    fn service_enabled(&self, _: &str, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}
/// 目的：登记处替身：读面全部失败（覆盖设置与 agent 读取的错误出口）。
struct BrokenRegistry;
macro_rules! dead {
    ($($name:ident($($arg:ty),*) => $ret:ty),* $(,)?) => {
        $(fn $name(&self $(, _: $arg)*) -> Result<$ret, String> { Err("登记处坏了".into()) })*
    };
}
impl RegistryOps for BrokenRegistry {
    fn agents(&self) -> Result<Vec<AgentView>, String> {
        Err("登记处坏了".into())
    }
    fn settings(&self) -> Result<AppSettings, String> {
        Err("登记处坏了".into())
    }
    dead! {
        providers() => Vec<ProviderView>, upsert_provider(&str, &str, &str) => (), remove_provider(&str) => bool,
        models() => Vec<ModelView>, core_model() => Option<String>, upsert_model(ModelDraft) => (),
        remove_model(&str) => bool, set_core_model(&str) => bool, pick_agents(&[String]) => Vec<AgentView>,
        upsert_agent(&str, &[String], &str, &str, &str) => (), remove_agent(&str) => bool,
        set_settings(AppSettings) => (), discover_models(&str) => Vec<String>,
        probe_model_tools(&str) => ProbeOutcome, probe_replay_shape(&str) => ReplayReport
    }
}
fn agent(modules: &[&str], model: Option<&str>) -> Agent {
    Agent {
        modules: modules.iter().map(|m| m.to_string()).collect(),
        model: model.map(str::to_string),
        note: String::new(),
        chara: String::new(),
    }
}

#[test]
fn startup_lines_read_facts_off_the_registry() {
    let (_h, ops) = ops_with(vec![module_of("a")], Vec::new());
    ui(false);
    let facts = startup_facts(&ops);
    assert_eq!(facts.len(), 4);
    assert!(facts[0].contains("agent 0"), "{}", facts[0]);
    assert!(facts[1].contains("模块 1") && facts[2].contains("档位 host"));
    assert!(facts[3].contains("模型 m"));
    assert!(
        term::capture(startup_tagline)
            .0
            .contains("终端转录中心 · v")
    );
    assert!(startup_status(&ops).contains("● 就绪 · 模型 m · 档位 host"));
    let models = ops.registry.models().expect("模型可读");
    assert_eq!(
        model_line(&models[0], Some("m")),
        "m（核心默认）  M → m  [p]"
    );
    assert!(!model_line(&models[0], Some("别的")).contains("（核心默认）"));
    assert_eq!(model_line(&models[0], None), "m  M → m  [p]");
    let p = ProviderView {
        id: "p".into(),
        base_url: "http://test".into(),
    };
    assert_eq!(provider_line(&p), "p  http://test");
    assert_eq!(model_label(None), "（核心默认）");
}
#[test]
fn startup_facts_fall_back_when_reads_fail() {
    let (_h, mut ops) = ops_with(vec![module_of("a")], Vec::new());
    ui(false);
    ops.modules = Arc::new(BrokenModules);
    ops.registry = Arc::new(BrokenRegistry);
    let facts = startup_facts(&ops);
    assert!(facts[0].contains("agent 0") && facts[1].contains("模块 0"));
    assert!(facts[2].contains("档位 host") && facts[3].contains("（未设定）"));
    assert!(startup_status(&ops).contains("（未设定）"));
}
#[test]
fn print_roster_covers_discovery_and_report_sections() {
    let (_h, ops) = ops_with(
        vec![module_of("a"), module_with_runtimes("b", &["qemu"])],
        Vec::new(),
    );
    ui(false);
    let lines = said(|| print_roster(&ops));
    let first = lines.first().expect("有发现行");
    assert!(first.starts_with("[发现]") && first.contains('a') && first.contains('b'));
    assert!(has(&lines, "a 的能力") && has(&lines, "运行能力 qemu"));
    let (_h, mut ops) = ops_with(vec![module_of("a")], Vec::new());
    ui(false);
    ops.core = Arc::new(FakeCore(Ok(report())));
    let rows = said(|| print_roster(&ops));
    let needles = concat!(
        "[拒收] 坏的清单|",
        "[运行包] 档位 vm；包库：qemu 9.2/9.1|",
        "[缺运行包] 模块 b 需要 qemu；把包放进依赖文件夹 runtimes/|",
        "[档位诊断] 运行包 qemu 有多个版本，需用户定版：9.2、9.1|",
        "[运行包拒收] 坏的包",
    );
    for needle in needles.split('|') {
        assert!(has(&rows, needle), "缺片段：{needle}");
    }
    assert!(!has(&rows, "包库里没有"), "缺包诊断由缺运行包分区说完");
    let (_h, ops) = ops_with(Vec::new(), Vec::new());
    ui(false);
    assert!(has(&said(|| print_roster(&ops)), "（无模块）"));
}
#[test]
fn print_roster_and_menu_report_read_errors() {
    let (_h, mut ops) = ops_with(vec![module_of("a")], Vec::new());
    ui(false);
    ops.modules = Arc::new(BrokenModules);
    assert!(has(&said(|| print_roster(&ops)), "[错误] 模块坏了"));
    let (_h, mut ops) = ops_with(vec![module_of("a")], Vec::new());
    ops.core = Arc::new(FakeCore(Err("报告坏了".into())));
    assert!(has(&said(|| print_roster(&ops)), "[错误] 报告坏了"));
    let (_h, mut ops) = ops_with(vec![module_of("a")], Vec::new());
    ops.registry = Arc::new(BrokenRegistry);
    assert!(has(&said(|| print_roster(&ops)), "[错误] 登记处坏了"));
    assert!(has(&said(|| print_menu(&ops)), "[错误] 登记处坏了"));
}
#[test]
fn print_menu_lists_agents_or_says_there_are_none() {
    let (_h, ops) = ops_with(Vec::new(), Vec::new());
    ui(false);
    let lines = said(|| print_menu(&ops));
    assert!(has(&lines, "可唤起 agent") && has(&lines, "（登记处还没有 agent"));
    let store = InMemorySettings::new();
    {
        let mut s = store.s.lock().expect("锁");
        s.agents.insert("甲".into(), agent(&["a"], Some("m")));
        s.agents.insert("乙".into(), agent(&["b"], None));
    }
    ui(false);
    let h = ConductorHandle::spawn(core_with_settings_modules(store, vec![module_of("a")]))
        .expect("起核心线程");
    let ops = Ops::from_handle(&h);
    let lines = said(|| print_menu(&ops));
    assert!(has(&lines, "a · 模型 m"), "agent 的模块与默认模型");
    assert!(has(&lines, "b · 模型 （核心默认）"), "没设模型用核心默认");
}
#[test]
fn rich_renders_the_event_vocabulary() {
    ui(true);
    let lines = said(|| render_rich(&vocabulary()));
    let needles = concat!(
        "通告|甲 开始干活，稍等|整理|待审|同意之后才开工|方案|[n1]|写测试|→ a|节点 n1|",
        "子会话 s1 · 负责人 a|r1|做完了|返工第 1 轮|改好了|验收|清单解析失败|原文|清单|",
        "第二条|第三条|全部通过，交付用户。|返工超限仍未通过，交用户裁决。|",
        "未能通过，详见上面的验收清单。|选一个|甲 在等你作答|甲 答了 c1：选了 o1|",
        "作废 2 张没答的卡（c1、c2）：会话停了",
    );
    for needle in needles.split('|') {
        assert!(has(&lines, needle), "富渲染缺片段：{needle}");
    }
    assert_eq!(lines.iter().filter(|l| l.contains("开始干活")).count(), 1);
    assert!(
        lines.iter().all(|l| !l.contains("上下文")),
        "用量写在状态行"
    );
    assert!(term::status_snapshot().contains("● 上下文 12.3k/128.0k（实计）"));
    let _ = term::capture(|| {
        render_rich(&[SessionEvent::Usage {
            usage: usage(false),
            line: 8,
        }])
    });
    assert!(
        term::status_snapshot().contains("（估算）"),
        "未实计时标估算"
    );
}
#[test]
fn plain_renders_flat_text() {
    ui(false);
    let lines = said(|| render_plain(&vocabulary()));
    let needles = concat!(
        "通告|---- 转录 ----|",
        "[压缩] 第 3 行之前已压成摘要；此后原样保留（仍可查看）：\n旧内容|",
        "[系统] 提醒|正文|[派发] 节点 n1 → 子会话 s1（负责人 a）|",
        "[整理] \n第一步|[待审] 方案如下，点「同意」才开工：\n方案|",
        "  - n1（写测试）负责人 a 依赖 []|[执行] [r1] 做完了|[执行·返工1] [r2] 改好了|",
        "[验收]|（清单解析失败，原文如下）\n原文|  [PASS] 清单 清单注|",
        "  [FAIL] 第二条 第二条注|  [待定] 第三条 第三条注|[交付] 全部通过，交付用户。|",
        "[裁决] 返工超限仍未通过，交用户裁决。|[交付] 未能通过。|",
        "[裁决] 有一张卡在等你：选一个（甲）——按提示作答|[裁决] 甲 答了 c1：选了 o1|",
        "[裁决] 作废 2 张没答的卡（c1、c2）：会话停了",
    );
    for needle in needles.split('|') {
        assert!(has(&lines, needle), "纯文本缺片段：{needle}");
    }
}
#[test]
fn render_picks_rich_for_tty_and_plain_otherwise() {
    ui(true);
    let lines = said(|| render(&[SessionEvent::Notice("彩色行".into())]));
    assert_ne!(lines[0], "彩色行", "TTY 走富渲染（带颜色转义）");
    assert!(has(&lines, "彩色行"));
    ui(false);
    assert_eq!(
        said(|| render(&[SessionEvent::Notice("素色行".into())])),
        vec!["素色行"]
    );
}
#[test]
fn print_tool_marks_results_and_trims_args() {
    ui(true);
    let ok = said(|| print_tool(&tool("a", true, "cmd  a   b")));
    assert!(has(&ok, "a.ls") && has(&ok, "cmd a b") && has(&ok, "✓"));
    let failed = said(|| print_tool(&tool("", false, &"x".repeat(100))));
    assert!(has(&failed, "ls") && has(&failed, "✗"));
    assert!(!has(&failed, &"x".repeat(65)), "长参数预览会被截断");
    assert_eq!(arg_preview(""), "");
    assert_eq!(arg_preview("  a \n b  "), "a b");
    assert!(width::cols(&arg_preview(&"y".repeat(100))) <= 64);
    assert_eq!(tokens(9_999), "9999");
    assert_eq!(tokens(10_000), "10.0k");
    assert_eq!(tokens(12_345), "12.3k");
    assert_eq!(check_mark("PASS").0, "✓");
    assert!(check_mark("PASS").1 == theme::Ink::Green);
    assert_eq!(check_mark("通过").0, "✓");
    assert_eq!(check_mark("fail: x").0, "✗");
    assert!(check_mark("fail: x").1 == theme::Ink::Red);
    assert_eq!(check_mark("错误").0, "✗");
    assert_eq!(check_mark("待定").0, "·");
    assert!(check_mark("待定").1 == theme::Ink::Dim);
}
