//! 目的：命令面门禁——`systools/commands.yaml` 是唯一声明，投影成 op 书后由同一套解析器消费。
//! 管：op 书自洽（重名、位置序号、枚举、相位与执行者）、手册覆盖、两相分派认得每条命令、启动相不再自写旗语词法。
//! 不管：手册排版（`src/tests/cli.rs`）与命令行为（各业务测试）。
//! 联动：`systools/commands.yaml`、`src/units/business/tools/domain/op.rs`、`src/presentation/cli/boot.rs`、`src/presentation/cli.rs`。

use crate::presentation::cli::catalog;
use crate::units::business::tools::api::{CommandBook, Op, OpBook, Phase, Runtime};
use std::sync::OnceLock;

/// 启动相的分派实现（只按名字取字段；旗语与位置参数由声明解析）。
const BOOT: &str = include_str!("../presentation/cli/boot.rs");
/// 入口的内部协议分派（这里不许再出现产品命令）。
const ENTRY_ARGS: &str = include_str!("../entry/args.rs");
/// 转录相命令各自的执行函数（取参与打印；一条命令一个函数）。
const COMMANDS: &str = include_str!("../presentation/cli/commands.rs");
/// 会话流命令的执行函数（发起会话、注册处命令）。
const FLOWS: &str = include_str!("../presentation/cli/flows.rs");
/// 转录中心的分派实现（转录相命令的落点：主循环与共用词法）。
const CLI: &str = concat!(
    include_str!("../presentation/cli.rs"),
    include_str!("../presentation/cli/shell.rs")
);

/// 目的：内部协议旗语（不进命令表）：只给测试、探针与守门子进程用。
const INTERNAL: &[&str] = &[
    "--proc-watchdog",
    "--fence-run",
    "--https-check",
    "--unpack",
    "--print-fence-env",
    "--fence-verify",
    "--fence-prepare",
    "--print-routes",
    "--fence-clean",
    "--probe-tools",
    "--probe-replay",
];

/// 跨命令修饰旗语（不是某条命令自己的旗语，故不写进命令表）。
const MODIFIERS: &[&str] = &["--json"];

/// 目的：命令表投影成的 op 书——门禁从它取证，不再自己维护一份清单。
fn ops() -> &'static OpBook {
    static OPS: OnceLock<OpBook> = OnceLock::new();
    OPS.get_or_init(|| {
        let book = CommandBook::builtin().expect("命令表可解析");
        OpBook::from_commands(book)
    })
}

/// 目的：某一相的命令（声明顺序）。
fn phase(phase: Phase) -> Vec<&'static Op> {
    ops().ops().iter().filter(|op| op.phase == phase).collect()
}

/// 目的：某一相的分组，按首次出现排序。
fn groups(phase: Phase) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for op in self::phase(phase) {
        if !out.contains(&op.group) {
            out.push(op.group.clone());
        }
    }
    out
}

/// 目的：从一段 Rust 源码里抠出所有 `"--xxx"` 旗语字面量（去重，保持出现顺序）。
fn flags_in(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 2 < chars.len() {
        if chars[i] == '"' && chars[i + 1] == '-' && chars[i + 2] == '-' {
            let mut j = i + 1;
            let mut name = String::new();
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') {
                name.push(chars[j]);
                j += 1;
            }
            if name.len() > 2 && !out.contains(&name) {
                out.push(name);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// 声明自身：能解析、无矛盾、两相都不为空、分组按首次出现排序。
#[test]
fn table_is_self_consistent() {
    let book = ops();
    assert!(book.problems().is_empty(), "{:?}", book.problems());
    assert_eq!(
        phase(Phase::Boot).len(),
        7,
        "启动相七条：doctor/provision/work-gc/storage/gc/build/setup"
    );
    assert_eq!(
        phase(Phase::Core).len(),
        18,
        "转录相十八条（module 命令已删：模块工具走 <模块> 词；modules 是清单）"
    );
    assert_eq!(groups(Phase::Boot), vec!["维护"]);
    let core = groups(Phase::Core);
    assert_eq!(core.first().map(String::as_str), Some("会话"));
    assert_eq!(core.last().map(String::as_str), Some("系统"));
}

/// 运行需求：只有真要常驻运行时的命令声明 services；启动相永远走轻量路径。
#[test]
fn only_runtime_commands_ask_for_services() {
    let mut services: Vec<&str> = ops()
        .ops()
        .iter()
        .filter(|op| op.runtime == Runtime::Services)
        .map(|op| op.id.as_str())
        .collect();
    services.sort_unstable();
    assert_eq!(
        services,
        vec![
            "collab", "diagnose", "explain", "proxy", "resident", "single", "web"
        ],
        "services 只给进会话/生成、Web、常驻服务与模块工具调用"
    );
    for op in phase(Phase::Boot) {
        assert!(
            op.runtime == Runtime::None,
            "{} 是启动相命令，不能声明 runtime: services",
            op.id
        );
    }
}

/// 相位与执行者自洽：启动相只可能是系统单元，转录相不许挂系统单元。
#[test]
fn phase_and_executor_agree() {
    let mut broken = Vec::new();
    for op in phase(Phase::Boot) {
        if op.executor.name() != "system" {
            broken.push(format!("{} 是启动相却是 {}", op.id, op.executor.name()));
        }
    }
    for op in phase(Phase::Core) {
        if op.executor.name() == "system" {
            broken.push(format!("{} 是转录相却挂着系统单元", op.id));
        }
    }
    assert!(broken.is_empty(), "{broken:?}");
}

/// 手册：启动命令一条不落（含它的写法），单条说明给出旗语与确认门槛。
#[test]
fn help_lists_every_boot_command() {
    let help = catalog::help_text(false);
    for op in phase(Phase::Boot) {
        let usage = catalog::entry_usage(op);
        assert!(help.contains(&usage), "手册漏了：{}", usage);
        assert!(help.contains(&op.brief), "手册漏了说明：{}", op.brief);
    }
    let gc = catalog::detail("--gc").expect("启动相命令有自己的说明");
    let gc_op = ops().find("gc").expect("gc");
    assert!(gc.contains(&catalog::entry_usage(gc_op)), "{gc}");
    assert!(gc.contains("--yes"), "旗语要列出来：{gc}");
    assert!(gc.contains("--category <string>"), "{gc}");
    assert!(gc.contains("确认："), "有写操作就要写确认门槛：{gc}");
    assert!(
        catalog::detail("--doctor")
            .expect("doctor")
            .contains("启动命令")
    );
}

/// 对齐：表里的启动命令启动相认得；启动相也不许再手写命令旗语（词法归声明与 `op::parse`）。
#[test]
fn boot_commands_match_boot_dispatch() {
    for op in phase(Phase::Boot) {
        let literal = format!("\"{}\"", op.id);
        assert!(
            BOOT.contains(&literal),
            "启动命令 {} 在 presentation/cli/boot.rs 里没有执行入口",
            op.id
        );
    }
    for flag in flags_in(BOOT) {
        assert!(
            INTERNAL.contains(&flag.as_str()) || MODIFIERS.contains(&flag.as_str()),
            "启动相里还留着命令旗语 {}（旗语由命令声明的参数派生，不该手写）",
            flag
        );
    }
}

/// 入口只剩内部协议：产品命令与产品旗语都不许再出现在 src/entry/args.rs。
#[test]
fn entry_keeps_only_internal_protocol() {
    for flag in flags_in(ENTRY_ARGS) {
        assert!(
            INTERNAL.contains(&flag.as_str()) || flag.starts_with("--fence-"),
            "src/entry/args.rs 里还留着产品旗语 {}（产品命令归启动相）",
            flag
        );
    }
}

/// 对齐：表里的转录相命令 CLI 认得，且补全与建议只从表里出。
#[test]
fn core_commands_match_cli_dispatch() {
    for op in phase(Phase::Core) {
        let literal = format!("\"{}\"", op.id);
        assert!(
            CLI.contains(&literal),
            "转录相命令 {} 在 presentation/cli.rs 里没有分派",
            op.id
        );
        assert!(
            catalog::complete(&format!("/{}", op.id)).contains(&format!("/{}", op.id)),
            "补全里没有 {}",
            op.id
        );
    }
    assert!(catalog::suggest("singel").as_deref() == Some("single"));
    assert!(catalog::suggest("porxy").as_deref() == Some("proxy"));
}

/// 菜单：`/` 起头且还没敲空格时按前缀列转录相命令——顺序按声明、别名也命中、说明不为空；
/// 普通消息、写完命令词带空格的、启动相写法都不给行（菜单只服务转录相命令）。
#[test]
fn menu_lists_core_commands_by_prefix() {
    let all = catalog::menu("/");
    let declared: Vec<&str> = phase(Phase::Core).iter().map(|op| op.id.as_str()).collect();
    assert_eq!(all.len(), declared.len(), "没有前缀时列出全部转录相命令");
    for (row, id) in all.iter().zip(&declared) {
        assert_eq!(row.name, format!("/{}", id), "顺序按命令表的声明");
        assert!(!row.brief.trim().is_empty(), "{} 没有一句说明", row.name);
    }
    assert_eq!(catalog::menu("/he").len(), 1, "前缀收窄到一条");
    assert_eq!(catalog::menu("/he")[0].name, "/help");
    assert_eq!(
        catalog::menu("/?")[0].name,
        "/help",
        "别名命中，展示的仍是命令词本身"
    );
    assert!(catalog::menu("hello").is_empty(), "消息不给菜单");
    assert!(catalog::menu("/web ").is_empty(), "写完命令词进入参数");
    assert!(catalog::menu("/--doctor").is_empty(), "启动相命令不在这里");
    assert!(catalog::menu("/nope").is_empty());
}

/// 机器回执：启动命令的说明里要写清 --json 会得到什么字段，且 --json 能被解析器认出来。
#[test]
fn boot_commands_declare_json_receipt() {
    for op in phase(Phase::Boot) {
        let json = op.json.as_deref().unwrap_or("");
        assert!(
            !json.trim().is_empty(),
            "启动命令 {} 没写 --json 的字段说明",
            op.id
        );
        let detail = catalog::detail(&format!("--{}", op.id)).expect("启动命令有自己的说明");
        assert!(detail.contains("等价 JSON"), "{}", detail);
        assert!(detail.contains(json), "说明里的字段要与表一致：{}", detail);
    }
}

/// 解析器：全局旗语与顺序无关——写在启动命令前面或后面效果一样，命令自己的旗语仍留在操作数里。
#[test]
fn global_flags_work_on_both_sides_of_a_boot_command() {
    let before = crate::presentation::cli::argv::parse(&["--json".to_string(), "--gc".to_string()])
        .expect("--json --gc 可解析");
    assert!(before.json);
    assert_eq!(before.command.as_deref(), Some("gc"));
    let after = crate::presentation::cli::argv::parse(&["--gc".to_string(), "--json".to_string()])
        .expect("--gc --json 可解析");
    assert_eq!(after.command.as_deref(), Some("gc"));
    assert!(after.json, "命令后的 --json 同样置位");
    assert!(
        after.operands.is_empty(),
        "全局旗语不进操作数：{:?}",
        after.operands
    );

    // 平台用例就是这样调的：命令在前、根旗在后，根必须真的被解析到（否则医生自检跑在错误的根上）。
    let rooted = crate::presentation::cli::argv::parse(&[
        "--doctor".to_string(),
        "--root".to_string(),
        "fixture".to_string(),
    ])
    .expect("--doctor --root fixture 可解析");
    assert_eq!(rooted.command.as_deref(), Some("doctor"));
    assert_eq!(rooted.root.as_deref(), Some("fixture"));
    assert!(rooted.operands.is_empty(), "{:?}", rooted.operands);

    // 命令自己的旗语与裸词按原顺序留给启动相（再由 op::parse 校验）。
    let gc = crate::presentation::cli::argv::parse(&[
        "--gc".to_string(),
        "--category=logs".to_string(),
        "--json".to_string(),
        "--yes".to_string(),
    ])
    .expect("--gc --category=logs 可解析");
    assert!(gc.json);
    assert_eq!(
        gc.operands,
        vec!["--category=logs".to_string(), "--yes".to_string()],
        "命令自己的旗语要原样留在操作数里"
    );

    // 命令后面的 --help 是这条命令自己的手册。
    let topic = crate::presentation::cli::argv::parse(&["--gc".to_string(), "--help".to_string()])
        .expect("--gc --help 可解析");
    assert_eq!(topic.help_topic.as_deref(), Some("gc"));
    assert!(!topic.help);

    // 命令放在 --help 后面一样算这条命令的手册（--help 先被看见）。
    let leading =
        crate::presentation::cli::argv::parse(&["--help".to_string(), "--gc".to_string()])
            .expect("--help --gc 可解析");
    assert_eq!(leading.command.as_deref(), Some("gc"));
    assert_eq!(leading.help_topic.as_deref(), Some("gc"));
    assert!(!leading.help);

    // 启动相之后的裸词也是它自己的操作数（工作名 / 模块 id）。
    let bare = crate::presentation::cli::argv::parse(&["--work-gc".to_string(), "dev".to_string()])
        .expect("--work-gc dev 可解析");
    assert_eq!(bare.command.as_deref(), Some("work-gc"));
    assert_eq!(bare.operands, vec!["dev".to_string()]);
    assert!(bare.positionals.is_empty(), "启动相之后不该再进位置参数");
}

/// 覆盖：\`systools/tools.yaml\` 的每条动作都有归属——要么在人的命令面（\`HUMAN_ACTIONS\`）里，
/// 要么由某个会话来源在会话里交出（内置面 / 模块工具 / 席位工具 / 工作区工具 / 会话类型的系统工具）。
///
/// 口径：人的命令面在 tools 单元的 \`is_human_action\` 里只有一处；呈现层入口检查对这份名单做，
/// 名单里少数只有工具形态、没有按钮的条目在下面登记豁免。
/// 新增动作忘了归属、或忘了给呈现层入口，这条会红。
#[test]
fn every_action_belongs_to_the_human_face_or_a_session_source() {
    use crate::units::business::tools::api as t;
    use crate::units::business::tools::api::HUMAN_ACTIONS;
    /// 人的命令面里有、但只有工具形态（没有呈现层按钮）的那几条。
    const TOOL_ONLY: &[&str] = &["read", "list", "grep", "shell", "compact"];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let systools =
        std::fs::read_to_string(root.join("systools").join("tools.yaml")).expect("工具表要读得到");
    let mut ids: Vec<String> = Vec::new();
    for line in systools.lines() {
        if let Some(rest) = line.strip_prefix("  ")
            && !rest.starts_with(' ')
            && let Some(name) = rest.strip_suffix(':')
        {
            ids.push(name.trim().to_string());
        }
    }
    assert!(!ids.is_empty(), "工具表里应当有动作");

    // 会话侧会交出的 id：内置面（含 submit_report）、工作区工具、段声明里的机制动作、会话类型的系统工具。
    let mut session: Vec<String> = t::names();
    session.extend(
        crate::units::business::workspace::api::WORK_TOOLS
            .iter()
            .map(|s| s.to_string()),
    );
    let decls = crate::tests::doubles::test_decls();
    for l in &decls.loops().loops {
        for c in &l.cast {
            session.extend(c.tools.iter().cloned());
        }
    }
    for p in &decls.profiles().profiles {
        session.extend(p.system_tools.iter().cloned());
    }
    let mut orphan: Vec<&str> = Vec::new();
    for id in &ids {
        if t::is_human_action(id) || session.contains(id) {
            continue;
        }
        orphan.push(id.as_str());
    }
    assert!(
        orphan.is_empty(),
        "这些动作既没有人的入口，也没有任何会话来源交出它：{:?}",
        orphan
    );

    let mut blob = String::new();
    let mut stack = vec![root.join("src").join("presentation")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("呈现层目录要读得到") {
            let path = entry.expect("目录项可读").path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs" | "js" | "html")
            ) {
                blob.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }

    let mut absent: Vec<&str> = Vec::new();
    for id in HUMAN_ACTIONS {
        if TOOL_ONLY.contains(id) {
            continue;
        }
        let quoted = format!("\"{}\"", id);
        let single = format!("'{}'", id);
        if !blob.contains(&quoted) && !blob.contains(&single) {
            absent.push(id);
        }
    }
    assert!(
        absent.is_empty(),
        "这些动作在人的命令面里，但呈现层没有入口，也没登记豁免：{:?}",
        absent
    );
}

/// 目的：一条命令声明的参数名（没有参数时为空）。
fn declared_of(op: &Op) -> Vec<String> {
    op.schema
        .params
        .as_ref()
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default()
}

/// 目的：从一个执行函数的函数体里取出它读过的参数名（text(args, "x") 与 num(args, "x")）。
fn reads_of(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for lead in ["text(args, \"", "num(args, \""] {
        let mut rest = body;
        while let Some(at) = rest.find(lead) {
            let after = &rest[at + lead.len()..];
            let Some(end) = after.find('"') else { break };
            let name = after[..end].to_string();
            if !out.contains(&name) {
                out.push(name);
            }
            rest = &after[end..];
        }
    }
    out
}

/// 目的：按函数名从源码里切出函数体（到列首的右花括号为止）。
fn body_of<'a>(sources: &'a str, name: &str) -> Option<&'a str> {
    let head = format!("fn {}(", name);
    let start = sources.find(&head)?;
    let tail = &sources[start..];
    let mut offset = 0;
    for line in tail.lines() {
        offset += line.len() + 1;
        if line == "}" && offset > head.len() {
            return Some(&tail[..offset]);
        }
    }
    None
}

/// 对齐：转录相命令的执行函数读的参数名必须在这一条命令的声明里。
#[test]
fn executors_only_read_the_params_their_command_declares() {
    let sources = format!("{}\n{}\n{}", COMMANDS, FLOWS, CLI);
    let mut checked = 0;
    for op in phase(Phase::Core) {
        let declared = declared_of(op);
        let snake = op.id.replace('-', "_");
        let mut found = false;
        for suffix in ["_cmd", "_flow"] {
            let name = format!("{}{}", snake, suffix);
            let Some(body) = body_of(&sources, &name) else {
                continue;
            };
            found = true;
            for read in reads_of(body) {
                assert!(
                    declared.contains(&read),
                    "命令 {} 的执行函数读参数 {}，但 systools/commands.yaml 没声明它",
                    op.id,
                    read
                );
            }
        }
        if found {
            checked += 1;
        }
    }
    assert!(
        checked >= 9,
        "至少九条转录相命令有专门的执行函数，实际 {}",
        checked
    );
}
