//! 目的：启动相命令的执行——把 `flags.command` 与它的操作数翻成系统维护能力的一次调用，并打回执、给退出码。
//! 管：操作数解析（--yes / --reset / --ensure / --category / --guest / --module 与裸词）、回执排版、退出码。
//! 不管：命令有哪些（在 systools/commands.yaml）、业务规则与确认语义（在 system 能力）、装配（在 entry::assembly）。
//! 联动：main 在工具链就位后调用；能力面由 entry::assembly::system 注入；两端一致性由 src/tests/commands.rs 锁死。

use crate::capabilities::storage::api::{CleanOutcome, GcReport, JobState};
use crate::capabilities::system::api::{
    BuildRequest, EnsureRequest, Fault, GcRequest, ResetRequest, System, WorkGcRequest,
};

/// 目的：一次启动相命令的入口——回执已经打完，只差退出码。
pub(crate) fn run(command: &str, operands: &[String], system: &dyn System) -> i32 {
    let args = match Args::parse(operands) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", e);
            return 2;
        }
    };
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
            Err(f) => fail("[存储]", f),
        },
        "provision" => provision(system, &args),
        "gc" => gc(system, &args),
        "work-gc" => work_gc(system, &args),
        "build" => build(system, &args),
        other => {
            eprintln!("这个启动命令还没有执行入口：{}", other);
            1
        }
    }
}

/// 目的：`--provision`：带 --reset 就还原虚拟根，带 --ensure 就补齐，都没有就报状态。
fn provision(system: &dyn System, args: &Args) -> i32 {
    if let Some(guest) = args.reset.clone() {
        let req = ResetRequest {
            guest,
            confirm: args.yes,
        };
        let mut on_line = |line: &str| eprintln!("[依赖补齐] {}", line);
        return match system.reset(req, &mut on_line) {
            Ok(report) => {
                print_gc(&report);
                0
            }
            Err(f) => fail("[依赖补齐]", f),
        };
    }
    if let Some(spec) = args.ensure.clone() {
        let req = EnsureRequest {
            spec: if spec.is_empty() { None } else { Some(spec) },
            confirm: args.yes,
        };
        let mut on_line = |line: &str| println!("[依赖补齐] {}", line);
        return match system.ensure(req, &mut on_line) {
            Ok(()) => {
                println!("[依赖补齐] 完成");
                0
            }
            Err(f) => fail("[依赖补齐]", f),
        };
    }
    match system.provision() {
        Ok(v) => {
            println!("{}", pretty(&v));
            0
        }
        Err(f) => fail("[依赖检查]", f),
    }
}

/// 目的：`--gc`：给了类别就清那一类，否则按策略回收；缺 --yes 时只预览（退出码 2）。
fn gc(system: &dyn System, args: &Args) -> i32 {
    if let Some(cat) = args.category.clone().filter(|_| !args.yes) {
        return gc_preview(system, &cat);
    }
    let req = GcRequest {
        kind: args.category.clone(),
        guest: args.guest.clone(),
        module: args.module.clone(),
        confirm: args.yes,
    };
    match system.gc(req) {
        Ok(CleanOutcome::Report(report)) => {
            print_gc(&report);
            0
        }
        Ok(CleanOutcome::Started(state)) => wait_job(system, state),
        Err(f) => fail("[存储]", f),
    }
}

/// 目的：带类别但不带 --yes 时的预览——只报这一类现在有多少，缺省退出码 2 表示「还差一次确认」。
fn gc_preview(system: &dyn System, category: &str) -> i32 {
    println!("[存储] 预览模式：确认后加 --yes 才会清理 {}", category);
    let report = match system.storage() {
        Ok(v) => v,
        Err(f) => return fail("[存储]", f),
    };
    let hit = report
        .get("categories")
        .and_then(|c| c.as_array())
        .and_then(|list| {
            list.iter().find(|c| {
                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                id == category || (category == "guest-bases" && id == category)
            })
        });
    match hit {
        Some(c) => {
            println!(
                "[存储] {}：{} 字节 / {} 项",
                c.get("label").and_then(|v| v.as_str()).unwrap_or(category),
                c.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                c.get("items")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0)
            );
            2
        }
        None => 2,
    }
}

/// 目的：后台作业起来后等它跑完（CLI 没有旁路事件通道，只能轮询状态快照）。
fn wait_job(system: &dyn System, mut state: JobState) -> i32 {
    let mut shown = 0usize;
    loop {
        for line in state.lines.iter().skip(shown) {
            println!("[存储] {}", line);
        }
        shown = shown.max(state.lines.len());
        if !state.running {
            break;
        }
        if let Some(e) = state.error.clone() {
            eprintln!("[存储] {}", e);
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        state = system.job();
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
fn work_gc(system: &dyn System, args: &Args) -> i32 {
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
            0
        }
        Err(f) => fail("[回收]", f),
    }
}

/// 目的：`--build`：在宿主或某个虚拟根里跑模块声明的构建。
fn build(system: &dyn System, args: &Args) -> i32 {
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
        Ok(b) if b.skipped => {
            println!("[构建] {} 在目标 {} 上已是最新，跳过", module, b.target);
            0
        }
        Ok(b) => {
            print!("{}", b.output);
            println!("[构建] {} 构建完成（目标 {}）", module, b.target);
            0
        }
        Err(f) => fail("[构建]", f),
    }
}

/// 目的：一次失败的收尾——打一行带域前缀的话，按失败自己的码退出。
fn fail(prefix: &str, fault: Fault) -> i32 {
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

/// 目的：启动相命令的操作数——跨命令修饰（--yes）与命令自己的旗语分开收。
#[derive(Debug, Default)]
struct Args {
    yes: bool,
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
