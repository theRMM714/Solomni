//! 目的：启动相命令的执行——把一次调用（命令名 + 操作数）翻成系统维护能力的一次调用，并打回执、给退出码。
//! 管：操作数解析（--yes / --reset / --ensure / --category / --guest / --module 与裸词）、写操作的就地确认、回执排版（人读或 --json）、退出码。
//! 不管：命令有哪些（在 systools/commands.yaml）、业务规则与确认门槛的判据（在 system 能力）、装配（在 entry::assembly）。
//! 联动：命令行由 main 调 `run`（没人可问，缺 --yes 只预览）；转录中心由 presentation::cli 调 `run_asked` 并接管确认；两端一致性由 src/tests/commands.rs 锁死。

use serde_json::{Value, json};

use crate::capabilities::storage::api::{CleanOutcome, GcReport, JobState};
use crate::capabilities::system::api::{
    BuildRequest, EnsureRequest, Fault, GcRequest, ResetRequest, SetupMode, System, WorkGcRequest,
};

/// 目的：写操作要人点头时，由谁来问——命令行上没人可问，转录中心里就地一问。
/// 约束：返回 false 表示没有明确点头（空输入、No、读不到输入都算没点头）。
pub(crate) trait Ask {
    /// 目的：这次调用背后有没有可问的人。
    /// 约束：没有时**不能问、也不能改写退出码**——命令行上「缺 --yes 只预览（退出码 2）」是既有契约。
    fn anyone(&self) -> bool;
    /// 参数：`question` 是一句包含后果与写法的问话（结尾的 [y/N] 由实现自己排版）。
    fn confirm(&mut self, question: &str) -> bool;
}

/// 目的：命令行入口的确认——没有交互对象，一律不点头（缺 --yes 就照旧只预览，退出码 2）。
pub(crate) struct NeverAsk;

impl Ask for NeverAsk {
    fn anyone(&self) -> bool {
        false
    }

    fn confirm(&mut self, _question: &str) -> bool {
        false
    }
}

/// 目的：一次启动相命令的入口——回执已经打完，只差退出码。
/// 参数：`json` 为真时回执改成一行 JSON（字段见 `systools/commands.yaml` 的 `json`）。
/// 约束：命令行上没人可问，写操作缺 `--yes` 就只给预览（行为与只有命令行入口时一致）。
pub(crate) fn run(command: &str, operands: &[String], system: &dyn System, json: bool) -> i32 {
    run_asked(command, operands, system, json, &mut NeverAsk, true)
}

/// 目的：一次启动相命令的完整入口——解析、确认、执行、回执。
/// 参数：`ask` 用来就地确认写操作；`writes` 为假表示此刻不该写（有会话在跑），只给预览。
/// 返回：0 成功；1 执行失败；2 用法或确认门槛不对。
pub(crate) fn run_asked(
    command: &str,
    operands: &[String],
    system: &dyn System,
    json: bool,
    ask: &mut dyn Ask,
    writes: bool,
) -> i32 {
    let mut args = match Args::parse(operands) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", e);
            return 2;
        }
    };
    let json = json || args.json;
    if needs_confirm(command, &args) {
        if !writes {
            // 有会话在跑：写操作会让它的工作目录跟着变，只给预览。
            eprintln!(
                "[提示] 有会话正在跑：{} 这次只给预览，等它跑完再来",
                command
            );
            args.yes = false;
            return run_once(command, system, &args, json);
        }
        if !args.yes {
            if !ask.anyone() {
                // 命令行：没人可问，照旧跑一遍不带确认的版本（只预览 / 只报门槛）。
                return run_once(command, system, &args, json);
            }
            // 先跑一遍不带确认的版本：预览 / dry-run / 门槛提示就是这一遍的输出。
            let _ = run_once(command, system, &args, json);
            if !ask.confirm(&question(command, &args)) {
                eprintln!("[提示] 已取消，没有执行（命令行里用 --yes 直接确认）");
                return 0;
            }
            args.yes = true;
        }
    }
    run_once(command, system, &args, json)
}

/// 目的：这条命令这次会不会写（写了才要确认）。
fn needs_confirm(command: &str, args: &Args) -> bool {
    match command {
        "provision" => args.reset.is_some() || args.ensure.is_some(),
        "gc" | "work-gc" => true,
        _ => false,
    }
}

/// 目的：确认问话——说清要动什么，不重复回执里已经打过的细节。
fn question(command: &str, args: &Args) -> String {
    match command {
        "gc" => match args.category.as_deref() {
            Some(c) => format!("确认清理存储类别 {}？[y/N]", c),
            None => "确认按策略回收存储？[y/N]".to_string(),
        },
        "work-gc" => match args.words.first() {
            Some(w) => format!("确认回收工作 {} 里不可达的对象？[y/N]", w),
            None => "确认回收？[y/N]".to_string(),
        },
        _ => match (args.reset.as_deref(), args.ensure.as_deref()) {
            (Some(guest), _) => format!("确认还原虚拟根 {}？会删掉它里面的状态并重建 [y/N]", guest),
            (_, Some(_)) => "确认从网络下载并补齐依赖？[y/N]".to_string(),
            _ => "确认执行？（会写项目内数据）[y/N]".to_string(),
        },
    }
}

/// 目的：按命令名分派到各命令的执行（已解析好操作数，确认与否已定）。
fn run_once(command: &str, system: &dyn System, args: &Args, json: bool) -> i32 {
    match command {
        "doctor" => {
            println!("{}", pretty(&system.doctor()));
            0
        }
        "storage" => match system.storage() {
            Ok(v) => {
                println!("{}", pretty(&v));
                0
            }
            Err(f) => fail("[存储]", f, json),
        },
        "provision" => provision(system, args, json),
        "gc" => gc(system, args, json),
        "work-gc" => work_gc(system, args, json),
        "build" => build(system, args, json),
        "setup" => setup(system, args, json),
        other => {
            eprintln!("这个启动命令还没有执行入口：{}", other);
            1
        }
    }
}

/// 目的：`--provision`：带 --reset 就还原虚拟根，带 --ensure 就补齐，都没有就报状态。
fn provision(system: &dyn System, args: &Args, json: bool) -> i32 {
    if let Some(guest) = args.reset.clone() {
        let req = ResetRequest {
            guest: guest.clone(),
            confirm: args.yes,
        };
        let mut lines: Vec<String> = Vec::new();
        let mut on_line = |line: &str| {
            lines.push(line.to_string());
            if !json {
                eprintln!("[依赖补齐] {}", line);
            }
        };
        return match system.reset(req, &mut on_line) {
            Ok(report) => {
                if json {
                    println!("{}", pretty(&json!({ "guest": guest, "report": report })));
                } else {
                    print_gc(&report);
                }
                0
            }
            Err(f) => fail("[依赖补齐]", f, json),
        };
    }
    if let Some(spec) = args.ensure.clone() {
        let req = EnsureRequest {
            spec: if spec.is_empty() { None } else { Some(spec) },
            confirm: args.yes,
        };
        let mut lines: Vec<String> = Vec::new();
        let mut on_line = |line: &str| {
            lines.push(line.to_string());
            if !json {
                println!("[依赖补齐] {}", line);
            }
        };
        return match system.ensure(req, &mut on_line) {
            Ok(()) => {
                if json {
                    println!("{}", pretty(&json!({ "ensured": true, "lines": lines })));
                } else {
                    println!("[依赖补齐] 完成");
                }
                0
            }
            Err(f) => fail("[依赖补齐]", f, json),
        };
    }
    match system.provision() {
        Ok(v) => {
            println!("{}", pretty(&v));
            0
        }
        Err(f) => fail("[依赖检查]", f, json),
    }
}

/// 目的：`--gc`：给了类别就清那一类，否则按策略回收；缺 --yes 时只预览（退出码 2）。
fn gc(system: &dyn System, args: &Args, json: bool) -> i32 {
    if let Some(cat) = args.category.clone().filter(|_| !args.yes) {
        return gc_preview(system, &cat, json);
    }
    let req = GcRequest {
        kind: args.category.clone(),
        guest: args.guest.clone(),
        module: args.module.clone(),
        confirm: args.yes,
    };
    match system.gc(req) {
        Ok(CleanOutcome::Report(report)) => {
            if json {
                println!(
                    "{}",
                    pretty(&serde_json::to_value(&report).unwrap_or_default())
                );
            } else {
                print_gc(&report);
            }
            0
        }
        Ok(CleanOutcome::Started(state)) => wait_job(system, state, json),
        Err(f) => fail("[存储]", f, json),
    }
}

/// 目的：带类别但不带 --yes 时的预览——只报这一类现在有多少，缺省退出码 2 表示「还差一次确认」。
fn gc_preview(system: &dyn System, category: &str, json: bool) -> i32 {
    let report = match system.storage() {
        Ok(v) => v,
        Err(f) => return fail("[存储]", f, json),
    };
    let hit = report
        .get("categories")
        .and_then(|c| c.as_array())
        .and_then(|list| {
            list.iter()
                .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(category))
        });
    if json {
        let body = json!({
            "preview": true,
            "category": category,
            "report": hit.cloned().unwrap_or(serde_json::Value::Null),
        });
        println!("{}", pretty(&body));
        return 2;
    }
    println!("[存储] 预览模式：确认后加 --yes 才会清理 {}", category);
    if let Some(c) = hit {
        println!(
            "[存储] {}：{} 字节 / {} 项",
            c.get("label").and_then(|v| v.as_str()).unwrap_or(category),
            c.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0),
            c.get("items")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        );
    }
    2
}

/// 目的：后台作业起来后等它跑完（CLI 没有旁路事件通道，只能轮询状态快照）。
fn wait_job(system: &dyn System, mut state: JobState, json: bool) -> i32 {
    let mut shown = 0usize;
    loop {
        if !json {
            for line in state.lines.iter().skip(shown) {
                println!("[存储] {}", line);
            }
        }
        shown = shown.max(state.lines.len());
        if !state.running {
            break;
        }
        if state.error.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        state = system.job();
    }
    if json {
        println!(
            "{}",
            pretty(
                &json!({ "job": { "running": state.running, "lines": state.lines, "error": state.error } })
            )
        );
    }
    match state.error {
        Some(e) => {
            eprintln!("[存储] {}", e);
            1
        }
        None => 0,
    }
}

/// 目的：`--work-gc`：回收一个工作里不再可达的内容对象（缺 --yes 只预览）。
fn work_gc(system: &dyn System, args: &Args, json: bool) -> i32 {
    let Some(work) = args.words.first().cloned() else {
        eprintln!("用法：solomni --work-gc <工作名> [--yes]");
        return 2;
    };
    let req = WorkGcRequest {
        work,
        confirm: args.yes,
    };
    match system.work_gc(req) {
        Ok(r) => {
            if json {
                let body = json!({
                    "objects": r.objects,
                    "bytes": r.bytes,
                    "reachableObjects": r.reachable_objects,
                    "reachableBytes": r.reachable_bytes,
                    "unreachableObjects": r.unreachable_objects,
                    "unreachableBytes": r.unreachable_bytes,
                    "ignoredFiles": r.ignored_files,
                    "applied": r.applied,
                    "removedObjects": r.removed_objects,
                    "removedBytes": r.removed_bytes,
                    "removedIgnored": r.removed_ignored,
                });
                println!("{}", pretty(&body));
            } else {
                println!(
                    "[回收] 对象 {} 个 / {} 字节；可达 {} / {}；不可达 {} / {}；杂项 {} 个",
                    r.objects,
                    r.bytes,
                    r.reachable_objects,
                    r.reachable_bytes,
                    r.unreachable_objects,
                    r.unreachable_bytes,
                    r.ignored_files
                );
                if r.applied {
                    println!(
                        "[回收] 已删除 {} 个 / {} 字节；清理杂项 {} 个",
                        r.removed_objects, r.removed_bytes, r.removed_ignored
                    );
                } else {
                    println!("[回收] 未删除（dry-run）；确认后加 --yes");
                }
            }
            0
        }
        Err(f) => fail("[回收]", f, json),
    }
}

/// 目的：`setup`：便携维护（PATH shim 与围栏机器级预授）——省略模式就只报状态。
/// 约束：状态是只读回执，人读与 `--json` 打同一份 JSON；写入类模式的进度行带 `[便携]` 前缀。
fn setup(system: &dyn System, args: &Args, json: bool) -> i32 {
    let raw = args.words.first().cloned().unwrap_or_default();
    let Some(mode) = SetupMode::parse(&raw) else {
        return fail(
            "[便携]",
            Fault::usage(format!(
                "未知的 setup 模式「{}」；可用：{}",
                raw,
                SetupMode::list()
            )),
            json,
        );
    };
    let mut on_line = |line: &str| {
        if !json {
            println!("[便携] {}", line);
        }
    };
    match system.setup(mode, &mut on_line) {
        Ok(v) => {
            if json {
                println!("{}", pretty(&v));
            } else if mode == SetupMode::Status {
                print_setup(&v);
            }
            0
        }
        Err(f) => fail("[便携]", f, json),
    }
}

/// 目的：`setup status` 的人读排版——平台与根、PATH 入口、围栏预授缺项，只从回执里取。
fn print_setup(v: &Value) {
    let text = |a: &str, b: &str| -> String {
        v.get(a)
            .and_then(|x| x.get(b))
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let flag = |a: &str, b: &str| -> bool {
        v.get(a)
            .and_then(|x| x.get(b))
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
    };
    println!(
        "[便携] 平台：{}  根：{}",
        v.get("platform")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
        v.get("root").and_then(|x| x.as_str()).unwrap_or_default()
    );
    println!(
        "[便携] PATH 入口：{}（目录在：{}；shim：{}；shim 最新：{}）",
        text("path", "bin"),
        if flag("path", "has_bin") {
            "是"
        } else {
            "否"
        },
        if flag("path", "shim_present") {
            "在"
        } else {
            "没写"
        },
        if flag("path", "shim_current") {
            "是"
        } else {
            "否"
        }
    );
    let missing = v
        .get("fence")
        .and_then(|f| f.get("missing"))
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    println!(
        "[便携] 围栏预授：{}（缺 {} 处）",
        if flag("fence", "ready") {
            "已就绪"
        } else {
            "未就绪"
        },
        missing
    );
}

/// 目的：`--build`：在宿主或某个虚拟根里跑模块声明的构建。
fn build(system: &dyn System, args: &Args, json: bool) -> i32 {
    let Some(module) = args.words.first().cloned() else {
        eprintln!("用法：solomni --build <模块 id> [--guest <虚拟根>]");
        return 2;
    };
    if args.guest.as_deref().is_some_and(|g| g.trim().is_empty()) {
        eprintln!("[构建] --guest 要给虚拟根名（`.vm/guest/<名>`）");
        return 2;
    }
    let req = BuildRequest {
        module: module.clone(),
        guest: args.guest.clone(),
    };
    match system.build(req) {
        Ok(b) if json => {
            println!(
                "{}",
                pretty(&json!({
                    "module": b.module,
                    "target": b.target,
                    "skipped": b.skipped,
                    "output": b.output,
                }))
            );
            0
        }
        Ok(b) if b.skipped => {
            println!("[构建] {} 在目标 {} 上已是最新，跳过", module, b.target);
            0
        }
        Ok(b) => {
            print!("{}", b.output);
            println!("[构建] {} 构建完成（目标 {}）", module, b.target);
            0
        }
        Err(f) => fail("[构建]", f, json),
    }
}

/// 目的：一次失败的收尾——打一行带域前缀的话，按失败自己的码退出；`--json` 时改成一行 JSON。
fn fail(prefix: &str, fault: Fault, json: bool) -> i32 {
    if json {
        eprintln!(
            "{}",
            pretty(&json!({ "error": fault.message, "code": fault.code, "domain": prefix }))
        );
        return fault.code;
    }
    eprintln!("{} {}", prefix, fault.message);
    fault.code
}

/// 目的：机器可读回执的排版（缩进 JSON，便于人读也便于管道取字段）。
fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// 目的：打印一次回收报告（人类可读；不打印机器路径）。
pub(crate) fn print_gc(r: &GcReport) {
    if r.actions.is_empty() {
        println!(
            "[存储] {}；当前 {} 字节",
            if r.applied {
                "没有可回收项"
            } else {
                "没有候选"
            },
            r.before_bytes
        );
        return;
    }
    for a in &r.actions {
        println!(
            "[存储] {} {}：{}（{}）{}",
            if a.applied { "回收" } else { "可回收" },
            a.category,
            a.label,
            a.bytes,
            a.error.clone().unwrap_or_else(|| a.reason.clone())
        );
    }
    println!("[存储] 释放 {} 字节；仍超限={}", r.freed_bytes, r.over_cap);
}

/// 目的：启动相命令的操作数——跨命令修饰（--yes / --json）与命令自己的旗语分开收。
#[derive(Debug, Default)]
struct Args {
    yes: bool,
    /// 目的：机器回执（写在命令后面也算：`solomni --gc --json`）。
    json: bool,
    reset: Option<String>,
    ensure: Option<String>,
    category: Option<String>,
    guest: Option<String>,
    module: Option<String>,
    /// 目的：不带旗语的取值（工作名 / 模块 id）。
    words: Vec<String>,
}

impl Args {
    /// 目的：解析启动相命令的操作数（`--flag=value` 与 `--flag value` 两种写法都认）。
    fn parse(operands: &[String]) -> Result<Args, String> {
        let mut args = Args::default();
        let mut i = 0;
        while i < operands.len() {
            let raw = &operands[i];
            let (name, inline) = match raw.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (raw.as_str(), None),
            };
            let take = |i: &mut usize| -> Option<String> {
                if let Some(v) = inline.clone() {
                    return Some(v);
                }
                let next = operands.get(*i + 1)?;
                if next.starts_with('-') {
                    return Some(String::new());
                }
                *i += 1;
                Some(next.clone())
            };
            match name {
                "--yes" => args.yes = true,
                "--json" => args.json = true,
                "--reset" => args.reset = take(&mut i),
                "--ensure" => args.ensure = Some(take(&mut i).unwrap_or_default()),
                "--category" => args.category = take(&mut i),
                "--guest" => args.guest = take(&mut i),
                "--module" => args.module = take(&mut i),
                other if other.starts_with('-') => {
                    return Err(format!("未知旗语 {}（--help 看全部）", other));
                }
                _ => args.words.push(raw.clone()),
            }
            i += 1;
        }
        Ok(args)
    }
}
