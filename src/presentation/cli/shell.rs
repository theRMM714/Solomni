//! 目的：一条命令的入口——词表先落到声明上（JSON），再按执行者交给业务面；命令行与转录中心共用这一处分派。
//! 管：命令名的归属（启动相 / 转录相 / 模块工具）、`--json` `--help` `--version` 三个修饰的摘取、模块清单与三级手册的打印。
//! 不管：词法规则与校验（由 `Op` 的声明派生，见 tools 能力的 op 模块）、手册排版（`cli/catalog.rs`）、业务决策（各入站面）。
//! 联动：`src/main.rs` 在核心装配后调一次；`cli.rs` 的提示符每读一行就调 `run_line`。

use super::*;
use crate::units::business::catalog::modules::api::{Module, Roster, ToolDecl};
use crate::units::business::runtime::api::{ActionCall, Caller};
use crate::units::business::system::api::System;
use crate::units::business::tools::api::{
    Executor, Op, Phase, Runtime, ToolSchema, arg_fault_text, parse_op, usage_named,
};
use crate::units::business::turn::turn::api::Output;
use serde_json::{Value, json};

/// 目的：这一次调用带进来的全局修饰：命令行与转录中心各自填，后面的分派只认这几个事实。
pub(crate) struct Opts {
    /// 目的：回执给机器读（`--json`，命令行与行内都有）。
    pub(crate) json: bool,
    /// 目的：要手册（`--help` / `-h`）。
    pub(crate) help: bool,
    /// 目的：要版本（`--version` / `-v`）。
    pub(crate) version: bool,
    /// 目的：在转录中心里跑：可以就地确认，`web` 用缺省端口，`exit` 真的退出。
    pub(crate) interactive: bool,
    /// 目的：转录中心里 `web` 不带端口时的缺省值。
    pub(crate) web_port: u16,
    /// 目的：启动目录收成的工作区根（None = 会话用产品内 work/）。
    pub(crate) workspace: Option<std::path::PathBuf>,
    /// 目的：没绑工作区的原因（启动目录不能当工作区时的一句话；`--no-workspace` 时为空）。
    pub(crate) workspace_skip: Option<String>,
}

/// 目的：一条命令跑完之后屏幕该干什么。
pub(crate) enum Outcome {
    /// 目的：跑完了，带退出码（命令行据此 exit，转录中心只当一行输出）。
    Done(i32),
    /// 目的：转 Web 转录中心（端口）。
    Web(u16),
    /// 目的：退出（只在转录中心里出现）。
    Exit,
}

/// 目的：把一行文本切成词：空白分隔，双引号 / 单引号内的空格保留（`--note "a b"` 是一个值）。
pub(crate) fn words(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 目的：转录中心的一次输入：切词、摘修饰，再走与命令行同一个分派。
pub(crate) fn run_line(faces: &Faces, system: &dyn System, line: &str, opts: &Opts) -> Outcome {
    let cut = words(line);
    let refs: Vec<&str> = cut.iter().map(String::as_str).collect();
    run_words(faces, system, &refs, opts)
}

/// 目的：一个词表切出来的事实：三个修饰的最终值，以及摘掉它们之后剩下的词（第一个是命令或模块头）。
/// 约束：分派与「要不要常驻运行时」共用这一份切分，两处不得各写一套（否则判据会漂移）。
pub(crate) struct Words<'a> {
    pub(crate) json: bool,
    pub(crate) help: bool,
    pub(crate) version: bool,
    pub(crate) rest: Vec<&'a str>,
}

/// 目的：切一次词：摘掉 `--json` / `--help` / `--version`，`--` 之后的词一律当值。
/// 返回：单个短横的多字符词原样返回给调用方报用法错（R8：短旗只认 -h / -v）。
fn split_words<'a>(
    words: &[&'a str],
    json: bool,
    help: bool,
    version: bool,
) -> Result<Words<'a>, &'a str> {
    let mut cut = Words {
        json,
        help,
        version,
        rest: Vec::new(),
    };
    let mut flags_done = false;
    for word in words {
        if !flags_done {
            match *word {
                "--" => {
                    flags_done = true;
                    cut.rest.push(word);
                    continue;
                }
                "--json" => {
                    cut.json = true;
                    continue;
                }
                "--help" | "-h" => {
                    cut.help = true;
                    continue;
                }
                "--version" | "-v" => {
                    cut.version = true;
                    continue;
                }
                _ => {
                    if short_flag(word) {
                        return Err(word);
                    }
                }
            }
        }
        cut.rest.push(*word);
    }
    Ok(cut)
}

/// 目的：这一次调用要不要常驻运行时（视图盘、存储自动任务、常驻服务、围栏授权）；判据只认命令声明里的 `runtime`，不另立名单：手册与版本永远不要，模块工具调用要。
/// 约束：不查模块清单——非命令词的无尾词只可能是模块手册或拼错的命令，两者都只打印，不执行任何东西。
pub(crate) fn needs_runtime(words: &[&str], help: bool, version: bool) -> bool {
    let Ok(cut) = split_words(words, false, help, version) else {
        return true;
    };
    if cut.help || cut.version {
        return false;
    }
    let Some(head) = cut.rest.first().copied() else {
        return true;
    };
    if let Some(op) = catalog::ops().find(head) {
        return op.runtime == Runtime::Services;
    }
    cut.rest.len() > 1
}

/// 目的：一个词表的分派：先认命令目录（启动相 / 转录相），再认模块 id 与模块工具。
/// 约束：`--` 之后只当值（否定数、以短横线开头的值都写在它后面）。
pub(crate) fn run_words(
    faces: &Faces,
    system: &dyn System,
    words: &[&str],
    opts: &Opts,
) -> Outcome {
    let cut = match split_words(words, opts.json, opts.help, opts.version) {
        Ok(cut) => cut,
        Err(word) => {
            println!(
                "[用法] 没有旗语 {}（短旗只认 -h / -v）；要旗语请用一个短横，如 --help",
                word
            );
            return Outcome::Done(2);
        }
    };
    let json = cut.json;
    let help = cut.help;
    let version = cut.version;
    let Some(head) = cut.rest.first().copied() else {
        if help {
            print!("{}", catalog::help_text(false));
        }
        return Outcome::Done(0);
    };
    let tail = &cut.rest[1..];
    if let Some(op) = catalog::ops().find(head) {
        return run_op(faces, system, op, tail, opts);
    }
    if let Some(module) = modules(faces).into_iter().find(|m| m.manifest.id == head) {
        return run_module(faces, &module, tail, json, help, version);
    }
    match catalog::suggest(head) {
        Some(near) => println!(
            "[提示] 未知命令或模块 {}；你是不是想用 {}？（help 看全部命令，modules 看全部模块）",
            head, near
        ),
        None => println!(
            "[提示] 未知命令或模块 {}（help 看全部命令；modules 看全部模块）",
            head
        ),
    }
    Outcome::Done(2)
}

/// 目的：单破折号的多字符词不是旗语（R8：短旗只认 -h / -v）；负数之类的值照收。
fn short_flag(word: &str) -> bool {
    word.len() > 1
        && word.starts_with('-')
        && !word.starts_with("--")
        && word.parse::<f64>().is_err()
}

/// 目的：命令目录里的一条：手册与版本先答（不需要参数写对），再解析参数、交给执行者。
fn run_op(faces: &Faces, system: &dyn System, op: &Op, tail: &[&str], opts: &Opts) -> Outcome {
    if opts.help {
        match catalog::detail(&op.id) {
            Some(text) => print!("{}", text),
            None => println!("[提示] 没有命令 {}", op.id),
        }
        return Outcome::Done(0);
    }
    if opts.version {
        println!(
            "[提示] 版本只有两层：solomni --version（产品）与 <模块> --version（模块）；{} 没有自己的版本",
            op.id
        );
        return Outcome::Done(2);
    }
    let args = match parse_op(op, tail) {
        Ok(v) => v,
        Err(fault) => {
            // 参数不符的回执自带签名与写法（与动作面同一套措辞）。
            println!("[用法] {}", arg_fault_text(&op.id, &op.schema, &fault));
            return Outcome::Done(2);
        }
    };
    match op.executor {
        Executor::System => Outcome::Done(run_system(faces, system, op, tail, opts)),
        Executor::Core => dispatch(faces, system, op, &args, opts),
        Executor::Ui => dispatch_ui(faces, op, &args, opts),
    }
}

/// 目的：启动相命令的执行：命令行上没人可问（缺 `--yes` 只预览），转录中心里就地一问。
fn run_system(faces: &Faces, system: &dyn System, op: &Op, tail: &[&str], opts: &Opts) -> i32 {
    if opts.interactive {
        return super::run_boot(faces, system, op, tail);
    }
    let operands: Vec<String> = tail.iter().map(|w| w.to_string()).collect();
    boot::run(op, &operands, system, opts.json)
}

/// 目的：转录相命令的业务分派：参数已经按声明校验过，这里只按名字装调用。
/// 约束：会话流（单 agent / 协作 / 代理）自己要进提示符，返回的是离开时的去向；其余命令跑完就回。
fn dispatch(faces: &Faces, system: &dyn System, op: &Op, args: &Value, opts: &Opts) -> Outcome {
    match op.id.as_str() {
        "single" => return single_flow(faces, system, args, opts),
        "collab" => {
            collab_flow(faces, args, opts);
            return Outcome::Done(0);
        }
        "proxy" => return proxy_flow(faces, system, None, opts),
        _ => {}
    }
    let json = opts.json;
    let code = match op.id.as_str() {
        "provider" => provider_flow(faces, args, json),
        "model" => model_flow(faces, args, json),
        "core" => core_flow(faces, args, json),
        "rewind" => rewind_cmd(faces, args, json),
        "work" => work_cmd(faces, args, json),
        "resident" => resident_cmd(faces, args, json),
        "secret" => secret_cmd(faces, args, json),
        "diagnose" => diagnose_cmd(faces, args, json),
        "explain" => explain_cmd(faces, args, json),
        other => {
            println!("[提示] {} 还没有接上执行者（装配时漏了）", other);
            2
        }
    };
    Outcome::Done(code)
}

/// 目的：纯呈现层自己的命令：手册、版本、退出、转 Web、重扫、模块清单。
fn dispatch_ui(faces: &Faces, op: &Op, args: &Value, opts: &Opts) -> Outcome {
    match op.id.as_str() {
        "help" => {
            let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
            match text(args, "topic") {
                Some(topic) => match catalog::detail(&topic) {
                    Some(text) => print!("{}", text),
                    None => match catalog::suggest(&topic) {
                        Some(near) => {
                            println!("[提示] 没有命令 {}；你是不是想查 {}？", topic, near)
                        }
                        None => println!("[提示] 没有命令 {}；help 看全部命令", topic),
                    },
                },
                None => print!("{}", catalog::help_text(all)),
            }
            Outcome::Done(0)
        }
        "version" => {
            println!("{}", catalog::version_text());
            Outcome::Done(0)
        }
        "modules" => Outcome::Done(modules_cmd(faces, opts.json)),
        "rescan" => {
            print_roster(faces);
            Outcome::Done(0)
        }
        "web" => match args.get("port").and_then(Value::as_u64) {
            Some(port) => Outcome::Web(port as u16),
            None if opts.interactive => Outcome::Web(opts.web_port),
            None => {
                println!("[提示] 命令行上用 --web 起界面（会话里才用 web 转过去）");
                Outcome::Done(0)
            }
        },
        "exit" => {
            if opts.interactive {
                Outcome::Exit
            } else {
                Outcome::Done(0)
            }
        }
        other => {
            println!("[提示] {} 还没有接上呈现动作（装配时漏了）", other);
            Outcome::Done(2)
        }
    }
}

/// 目的：模块公地的清单：合法模块（带版本与启用态）+ 拒收原因（校验不是挑选，都要如实显示）。
fn modules_cmd(faces: &Faces, json: bool) -> i32 {
    let roster = match faces.modules.roster() {
        Ok(r) => r,
        Err(e) => {
            println!("[错误] {}", e);
            return 1;
        }
    };
    let enabled = |id: &str| faces.modules.module_enabled(id).unwrap_or(false);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&listing_json(&roster, &enabled)).unwrap_or_default()
        );
        return 0;
    }
    print!("{}", listing_text(&roster, &enabled));
    0
}

/// 目的：清单的机器形状（`--json`）：每项 id/版本/启用态/一句话说明，拒收原因单独一组。
pub(crate) fn listing_json(roster: &Roster, enabled: &dyn Fn(&str) -> bool) -> Value {
    let modules: Vec<Value> = roster
        .modules
        .iter()
        .map(|m| {
            json!({
                "id": m.manifest.id,
                "version": m.manifest.version,
                "enabled": enabled(&m.manifest.id),
                "brief": m.manifest.brief,
            })
        })
        .collect();
    json!({ "modules": modules, "rejected": roster.rejected })
}

/// 目的：清单正文：模块、版本、启用态、一句话说明，拒收原因单独列（校验不是挑选）。
pub(crate) fn listing_text(roster: &Roster, enabled: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    if roster.modules.is_empty() {
        out.push_str("（没有可用模块）\n");
    }
    for m in &roster.modules {
        out.push_str(&format!(
            "  {:<20} {:<10} {:<6} {}\n",
            m.manifest.id,
            m.manifest.version,
            if enabled(&m.manifest.id) {
                "启用"
            } else {
                "停用"
            },
            first_line(&m.manifest.brief)
        ));
    }
    for reason in &roster.rejected {
        out.push_str(&format!("  （拒收）{}\n", reason));
    }
    if !roster.modules.is_empty() {
        out.push_str("用法：solomni <模块> 看手册；solomni <模块> <工具> [参数] 直接调用\n");
    }
    out
}

/// 目的：清单里的模块（每次重扫——清单即事实，不缓存）。
fn modules(faces: &Faces) -> Vec<Module> {
    match faces.modules.roster() {
        Ok(r) => r.modules,
        Err(e) => {
            println!("[错误] {}", e);
            Vec::new()
        }
    }
}

/// 目的：模块入口的三级分派：`<模块>` 手册、`<模块> --version` 版本、`<模块> <工具>` 调用。
fn run_module(
    faces: &Faces,
    module: &Module,
    tail: &[&str],
    json: bool,
    help: bool,
    version: bool,
) -> Outcome {
    let id = module.manifest.id.as_str();
    if tail.is_empty() {
        if version {
            if json {
                println!(
                    "{}",
                    json!({ "id": id, "version": module.manifest.version })
                );
            } else {
                println!("{} {}", id, module.manifest.version);
            }
            return Outcome::Done(0);
        }
        return Outcome::Done(module_manual(faces, module, json));
    }
    let tool = tail[0];
    let Some(decl) = module.manifest.tools.get(tool) else {
        if help {
            println!(
                "[提示] 模块 {} 没有工具 {}（solomni {} 看手册）",
                id, tool, id
            );
            return Outcome::Done(2);
        }
        println!(
            "[提示] 模块 {} 没有工具 {}；看手册：solomni {}",
            id, tool, id
        );
        return Outcome::Done(2);
    };
    if version {
        // 版本只有两层：工具没有自己的版本，别让「render report -v」看起来像模块版本。
        println!("[提示] 工具的版本就是模块版本：solomni {} --version", id);
        return Outcome::Done(2);
    }
    if help {
        return Outcome::Done(tool_manual(module, tool, decl, json));
    }
    let op = tool_op(module, tool, decl);
    let words = &tail[1..];
    let (workspace, words) = take_workspace(words);
    let head = format!("{} {}", module.manifest.id, tool);
    let mut args = match tool_args(decl, &op, &head, &words) {
        Ok(v) => v,
        Err(msg) => {
            println!("{}", msg);
            return Outcome::Done(2);
        }
    };
    if let Some(ws) = workspace
        && let Value::Object(map) = &mut args
    {
        map.insert("workspace".to_string(), Value::String(ws));
    }
    Outcome::Done(act_and_print(
        faces,
        ActionCall {
            id: op.id.clone(),
            args,
            caller: Caller::User,
            out: Output::Final,
        },
        json,
    ))
}

/// 目的：模块手册：版本、状态、工具表、用法，以及给 agent 的系统说明原文。
fn module_manual(faces: &Faces, module: &Module, json: bool) -> i32 {
    let enabled = faces
        .modules
        .module_enabled(&module.manifest.id)
        .unwrap_or(false);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manual_json(module, enabled)).unwrap_or_default()
        );
    } else {
        print!("{}", manual_text(module, enabled));
    }
    0
}

/// 目的：模块手册的机器形状（`--json`）：清单事实原样，不做排版。
pub(crate) fn manual_json(module: &Module, enabled: bool) -> Value {
    let m = &module.manifest;
    let tools: Vec<Value> = m
        .tools
        .iter()
        .map(|(name, t)| {
            json!({
                "name": name,
                "desc": t.desc,
                "parallel": t.parallel,
                "params": t.params,
            })
        })
        .collect();
    json!({
        "id": m.id,
        "version": m.version,
        "brief": m.brief,
        "enabled": enabled,
        "system": m.system,
        "tools": tools,
    })
}

/// 目的：模块手册的正文：`id 版本 — 说明`、状态、给 agent 的系统说明、工具表与用法。
pub(crate) fn manual_text(module: &Module, enabled: bool) -> String {
    let m = &module.manifest;
    let mut out = format!("{} {} — {}\n", m.id, m.version, first_line(&m.brief));
    out.push_str(&format!(
        "状态：{}\n",
        if enabled {
            "已启用"
        } else {
            "已停用（solomni modules 看全部）"
        }
    ));
    if !m.system.trim().is_empty() {
        out.push('\n');
        out.push_str("给 agent 的系统说明：\n");
        for line in m.system.lines() {
            out.push_str(&format!("  {}\n", line));
        }
    }
    out.push('\n');
    if m.tools.is_empty() {
        out.push_str("工具：没有声明（只能经 AI 使用）\n");
    } else {
        out.push_str("工具：\n");
        for (name, t) in &m.tools {
            out.push_str(&format!("  {:<16} {}\n", name, first_line(&t.desc)));
        }
        out.push('\n');
        out.push_str(&format!(
            "用法：solomni {0} <工具> [参数]（solomni {0} <工具> --help 看参数）\n",
            m.id
        ));
    }
    out
}

/// 目的：单个工具的手册：用法串与参数表都由声明派生（不再手写）。
fn tool_manual(module: &Module, name: &str, decl: &ToolDecl, json: bool) -> i32 {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&tool_json(module, name, decl)).unwrap_or_default()
        );
    } else {
        print!("{}", tool_text(module, name, decl));
    }
    0
}

/// 目的：工具手册的机器形状（`--json`）：模块版本、说明、派生用法、参数表。
pub(crate) fn tool_json(module: &Module, name: &str, decl: &ToolDecl) -> Value {
    let m = &module.manifest;
    let op = tool_op(module, name, decl);
    json!({
        "module": m.id,
        "version": m.version,
        "tool": name,
        "desc": decl.desc,
        "usage": usage_named(&op, &format!("{} {}", m.id, name)),
        "parallel": decl.parallel,
        "params": decl.params,
    })
}

/// 目的：工具手册的正文：`模块 工具 · 版本`、说明、派生用法、参数表（没有参数就说清怎么给）。
pub(crate) fn tool_text(module: &Module, name: &str, decl: &ToolDecl) -> String {
    let m = &module.manifest;
    let op = tool_op(module, name, decl);
    let mut out = format!("{} {} · {}\n", m.id, name, m.version);
    if !decl.desc.trim().is_empty() {
        out.push_str(decl.desc.trim());
        out.push('\n');
    }
    out.push_str(&format!(
        "用法：solomni {}\n",
        usage_named(&op, &format!("{} {}", m.id, name))
    ));
    match &decl.params {
        None => out.push_str("参数：整段 JSON（省略 = {}）\n"),
        Some(map) if map.is_empty() => out.push_str("参数：无\n"),
        Some(map) => {
            out.push_str("参数：\n");
            for (param, schema) in map {
                out.push_str(&format!("  {}\n", catalog::param_row(param, schema)));
            }
        }
    }
    out.push_str(&format!("模块版本与系统说明：solomni {}\n", m.id));
    out
}

/// 目的：模块工具的词表落成 JSON：声明了参数就按声明解析，没声明就只收整段 JSON。
/// 返回：成功是参数对象；失败是可以直接打出去的整句（用法按命令行写法，不按动作 id）。
pub(crate) fn tool_args(
    decl: &ToolDecl,
    op: &Op,
    head: &str,
    words: &[&str],
) -> Result<Value, String> {
    match &decl.params {
        None => {
            let raw = words.join(" ");
            if raw.trim().is_empty() {
                Ok(json!({}))
            } else {
                serde_json::from_str::<Value>(&raw)
                    .map_err(|e| format!("[错误] 这条工具的参数是整段 JSON：{}", e))
            }
        }
        Some(_) => parse_op(op, words)
            .map_err(|fault| format!("[用法] {}", arg_fault_text(head, &op.schema, &fault))),
    }
}

/// 目的：把模块的一个工具套成 op：解析、用法与校验因此与命令表走同一套规则。
pub(crate) fn tool_op(module: &Module, name: &str, decl: &ToolDecl) -> Op {
    Op {
        id: format!("module.{}.{}", module.manifest.id, name),
        aliases: Vec::new(),
        group: "模块".to_string(),
        brief: decl.desc.clone(),
        phase: Phase::Core,
        executor: Executor::Core,
        confirm: false,
        // 模块工具真要起被围栏的进程：一律按需要常驻运行时处理。
        runtime: Runtime::Services,
        schema: ToolSchema {
            desc: decl.desc.clone(),
            params: decl.params.clone(),
            parallel: decl.parallel,
        },
        json: None,
    }
}

/// 目的：从模块工具的词表里摘出 `--workspace`（机制参数：交给执行面定工作目录，不进模块参数表）。
fn take_workspace<'a>(words: &'a [&'a str]) -> (Option<String>, Vec<&'a str>) {
    let mut out: Vec<&str> = Vec::new();
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        if let Some(value) = word.strip_prefix("--workspace=") {
            found = Some(value.to_string());
            i += 1;
            continue;
        }
        if word == "--workspace" {
            if let Some(value) = words.get(i + 1) {
                found = Some((*value).to_string());
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(word);
        i += 1;
    }
    (found, out)
}
