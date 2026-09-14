//! 终端转录中心：解析命令 → 调核心门面 → 渲染事件流。
//! 只做解析与渲染，不做业务决策；Web 前端与它并列、共用同一门面与事件词汇。

use crate::core::agents::AgentView;
use crate::core::{AgentInstance, CollabStep, Core, Live, Pending, SessionEvent, WorkMode, WorkSpec};
use crate::presentation::web::DEFAULT_PORT;
use std::io::Write;

/// 离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

pub fn run(mut core: Core) -> (Core, CliExit) {
    println!("Solomni 核心编排者（转录中心）");
    print_roster(&core);

    loop {
        print_menu(&core);
        print!("> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim().to_string();
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "single" => single_flow(&mut core, &arg),
            "collab" => collab_flow(&mut core, &arg),
            "provider" => provider_flow(&mut core, &arg),
            "model" => model_flow(&mut core, &arg),
            "core" => core_flow(&mut core, &arg),
            "rescan" => print_roster(&core),
            // 转入 Web 转录中心：接受 webui / -webUI（启动参数也这么写），可选端口。
            "webui" | "-webui" | "web" | "-web" => {
                let port = arg.parse::<u16>().unwrap_or(DEFAULT_PORT);
                return (core, CliExit::Web(port));
            }
            "exit" => break,
            "" => continue,
            _ => println!("[提示] 未知命令 {}（Web 界面用 webui；退出用 exit）", cmd),
        }
    }
    println!("再见。");
    (core, CliExit::Exit)
}

fn print_roster(core: &Core) {
    let roster = core.scan();
    // 运行能力报告与模块清单同源：按默认执行档位如实报（缺包不是崩溃，工具按档位不可用）。
    let report = core.runtime_report(core.app_settings().tier);
    println!(
        "[发现] {}",
        if roster.modules.is_empty() {
            "（无模块）".to_string()
        } else {
            roster.modules.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>().join(" · ")
        }
    );
    for m in &roster.modules {
        println!("  {:<14} {}", m.manifest.id, first_line(&m.manifest.brief));
    }
    for r in &report.rejected {
        println!("[拒收] {}", r);
    }
    for (id, caps) in &report.declared {
        println!("  {:<14} 运行能力 {}", id, caps.join("、"));
    }
    if !report.available.is_empty() {
        let lib = report
            .available
            .iter()
            .map(|(id, vs)| format!("{} {}", id, vs.join("/")))
            .collect::<Vec<_>>()
            .join(" · ");
        println!("[运行包] 档位 {}；包库：{}", report.tier, lib);
    }
    for (id, caps) in &report.missing {
        println!("[缺运行包] 模块 {} 需要 {}；把包放进依赖文件夹 runtimes/（契约见 RUNTIME_SPEC.md）", id, caps.join("、"));
    }
    // 虚拟机档的诊断：缺包之外（多版本未定版 / 定版不存在 / 路径冲突）会挡住「开始」，在这里如实说明。
    let hard: Vec<crate::core::exec::Diagnosis> = report
        .diagnoses
        .iter()
        .filter(|d| !matches!(d, crate::core::exec::Diagnosis::Missing { .. }))
        .cloned()
        .collect();
    if !hard.is_empty() {
        println!("[档位诊断] {}（虚拟机档要先解决这些才能开始会话）", crate::core::exec::diagnose_text(&hard));
    }
    for r in &report.rejected_packages {
        println!("[运行包拒收] {}", r);
    }
}

fn print_menu(core: &Core) {
    let agents = core.agent_views();
    println!("\n可唤起 agent（发言席只有 agent；模块是它的能力包）：");
    if agents.is_empty() {
        println!("  （登记处还没有 agent：到 Web 界面「设置 → agent 管理」建一个）");
    }
    for a in &agents {
        println!("  {:<14} {} · 模型 {}", a.name, a.modules.join(" + "), model_label(a.model.as_deref()));
    }
    println!("命令：single [agent名…] | collab [agent名…|?] | provider list|add|rm|discover | model list|add|rm | core <模型id> | rescan | webui | exit");
}

/// 模型标签（CLI 展示文案；核心默认是登记处的概念，不是提示词）。
fn model_label(model: Option<&str>) -> String {
    model.map(|m| m.to_string()).unwrap_or_else(|| "（核心默认）".to_string())
}

/// 点名已存 agent：CLI 只认登记处的名字（不再直接点模块）；不存在 / 登记处为空都明确报错。
fn named_agents(core: &Core, arg: &str) -> Result<Vec<AgentView>, String> {
    let views = core.agent_views();
    if views.is_empty() {
        return Err("登记处还没有 agent：请先到 Web 界面「设置 → agent 管理」建一个（CLI 不再直接点模块）".to_string());
    }
    let names: Vec<String> = arg
        .split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let mut out = Vec::new();
    for n in names {
        match views.iter().find(|a| a.name == n) {
            Some(a) => out.push(a.clone()),
            None => {
                return Err(format!(
                    "无此 agent：{}（现有：{}）",
                    n,
                    views.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" · ")
                ))
            }
        }
    }
    if out.is_empty() {
        return Err("没有点名任何 agent".to_string());
    }
    Ok(out)
}

// ---------- 事件渲染：CLI 与 Web 前端同源 ----------

fn render(events: &[SessionEvent]) {
    for e in events {
        match e {
            SessionEvent::Notice(n) => println!("{}", n),
            SessionEvent::Transcript(lines) => {
                println!("---- 转录 ----");
                for l in lines {
                    println!("{}", l.line);
                }
            }
            SessionEvent::DiscussionDone { .. } => {}
            SessionEvent::Plan(p) => println!("[整理] \n{}", p),
            SessionEvent::Report { id, text, rework } => {
                if *rework > 0 {
                    println!("[执行·返工{}] [{}] {}", rework, id, text);
                } else {
                    println!("[执行] [{}] {}", id, text);
                }
            }
            SessionEvent::Review { items, raw } => {
                println!("[验收]");
                if items.is_empty() {
                    println!("（清单解析失败，原文如下）\n{}", raw);
                }
                for i in items {
                    println!("  [{}] {} {}", i.status.to_uppercase(), i.item, i.note);
                }
            }
            SessionEvent::Delivery { ok, over_rework } => {
                if *ok {
                    println!("[交付] 全部通过，交付用户。");
                } else if *over_rework {
                    println!("[裁决] 返工超限仍未通过，交用户裁决。");
                } else {
                    println!("[交付] 未能通过。");
                }
            }
            SessionEvent::Ended => {}
            // 流式增量与工具调用实时事件都是短暂事件，终端不在流中渲染（最终行会到）。
            SessionEvent::Delta { .. } | SessionEvent::ToolCall(_) => {}
        }
    }
}

/// 工作名在当前进程内唯一：重名时追加序号（CLI 便捷；Web 由用户自己取名）。
fn unique_name(core: &Core, base: &str) -> String {
    if !core.session_exists(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let cand = format!("{}-{}", base, n);
        if !core.session_exists(&cand) {
            return cand;
        }
        n += 1;
    }
}

// ---------- 形态一：单 agent（模块数不限） ----------

fn single_flow(core: &mut Core, arg: &str) {
    let views = core.agent_views();
    if views.is_empty() {
        println!("[错误] 登记处还没有 agent：请先到 Web 界面「设置 → agent 管理」建一个（CLI 不再直接点模块）");
        return;
    }
    // 点名 1 个 = 直接用该 agent（模块数不限）；点名多个 = 把那几个的模块并成一个；无参 = 把登记处全部并成一个。
    let merge = |list: &[AgentView]| -> Vec<String> {
        let mut merged: Vec<String> = Vec::new();
        for a in list {
            for id in &a.modules {
                if !merged.contains(id) {
                    merged.push(id.clone());
                }
            }
        }
        merged
    };
    let (name, modules, model, transient) = if arg.trim().is_empty() {
        ("组合".to_string(), merge(&views), None, true)
    } else {
        let picked = match named_agents(core, arg) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        };
        if picked.len() == 1 {
            let a = &picked[0];
            (a.name.clone(), a.modules.clone(), a.model.clone(), false)
        } else {
            ("组合".to_string(), merge(&picked), None, true)
        }
    };
    let spec = WorkSpec {
        name: unique_name(core, "single"),
        mode: WorkMode::Single,
        agents: vec![AgentInstance { name, transient, modules, model }],
        task: None,
        delegate: false,
    };
    let (sid, open) = match core.create_work(spec) {
        Ok(o) => (o.sid, o.events),
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    render(&open);
    println!("（单 agent {} —— 输入消息，空行结束会话）", sid);
    let mut noop = |_e: SessionEvent| {};
    let mut live = Live { stream: false, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), emit: &mut noop };
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        match core.single_say(&sid, &say, &mut live) {
            Ok(events) => render(&events),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

// ---------- 模式三：协作（按核心 pending 驱动） ----------

fn collab_flow(core: &mut Core, arg: &str) {
    let trimmed = arg.trim();
    let delegate = trimmed.is_empty() || trimmed == "?";
    let task = prompt("需求>");
    // 代拟（无参或 ?）= 核心拟名单；点名 = 用登记处里的那几个 agent。
    let agents: Vec<AgentInstance> = if delegate {
        Vec::new()
    } else {
        match named_agents(core, trimmed) {
            Ok(l) => l
                .into_iter()
                .map(|a| AgentInstance { name: a.name, transient: false, modules: a.modules, model: a.model })
                .collect(),
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let spec = WorkSpec {
        name: unique_name(core, "collab"),
        mode: WorkMode::Collab,
        agents,
        task: Some(task),
        delegate,
    };
    let sid = match core.create_work(spec) {
        Ok(o) => {
            render(&o.events);
            o.sid
        }
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };

    // 名单确认（代拟路径）：把核心填好的表单逐行打出来，再问。
    if matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmSlate))) {
        match core.collab_slate(&sid) {
            Ok(list) => {
                println!("[代拟] 核心拟的名单：");
                for a in list {
                    println!(
                        "  {}：模块 {} · 模型 {} · {}",
                        a.name,
                        a.modules.join(" + "),
                        model_label(a.model.as_deref()),
                        if a.transient { "组装（临时）" } else { "复用已存 agent" }
                    );
                }
            }
            Err(e) => println!("[提示] 取名单失败：{}", e),
        }
        let ok = prompt("确认名单？（yes 开始 / 其他取消）");
        match core.collab_continue(&sid, CollabStep::ConfirmSlate, &ok) {
            Ok(events) => render(&events),
            Err(e) => println!("[错误] {}", e),
        }
    }
    // 开始确认。
    if matches!(core.collab_pending(&sid), Ok(Some(Pending::ConfirmBegin))) {
        let ans = prompt("开始讨论？（yes / yes,allow：授权小组自裁细节）");
        match core.collab_continue(&sid, CollabStep::Begin, &ans) {
            Ok(events) => render(&events),
            Err(e) => println!("[错误] {}", e),
        }
    }
    // ask 循环（每次回答后可能接新的请教）。
    while matches!(core.collab_pending(&sid), Ok(Some(Pending::Ask { .. }))) {
        if let Ok(Some(Pending::Ask { member, question })) = core.collab_pending(&sid) {
            println!("[请教] {}：{}", member, question);
            let ans = prompt("你的回答（回车 = 无补充，继续）>");
            match core.collab_continue(&sid, CollabStep::Answer, &ans) {
                Ok(events) => render(&events),
                Err(e) => {
                    println!("[错误] {}", e);
                    break;
                }
            }
        }
    }
}

// ---------- 登记处管理（密钥只在核心层进出） ----------

fn provider_flow(core: &mut Core, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let sub = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match sub {
        "list" | "" => {
            let lines = core.provider_lines();
            if lines.is_empty() {
                println!("（无供应商）用 provider add <id> <base_url> <api_key> 添加");
            }
            for l in lines {
                println!("  {}", l);
            }
        }
        "add" | "key" => {
            let w: Vec<&str> = rest.split_whitespace().collect();
            if w.len() < 3 {
                println!("[错误] 用法：provider add <id> <base_url> <api_key>");
                return;
            }
            match core.provider_upsert(w[0], w[1], w[2]) {
                Ok(()) => println!("[登记] {} 已保存（0600）", w[0]),
                Err(e) => println!("[错误] {}", e),
            }
        }
        "rm" => match core.provider_remove(rest) {
            Ok(true) => println!("[移除] {}", rest),
            Ok(false) => println!("[错误] 无此供应商：{}", rest),
            Err(e) => println!("[错误] {}", e),
        },
        "discover" => match core.discover_models(rest) {
            Ok(models) => println!("[发现] {}：{}", rest, models.join(" · ")),
            Err(e) => println!("[错误] {}", e),
        },
        _ => println!("[错误] 用法：provider list|add|rm|discover …"),
    }
}

fn model_flow(core: &mut Core, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let sub = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match sub {
        "list" | "" => {
            let lines = core.model_lines();
            if lines.is_empty() {
                println!("（无模型）用 model add <id> <展示名> <实际模型串> <供应商id> [note] 添加");
            }
            for l in lines {
                println!("  {}", l);
            }
        }
        "add" => {
            let w: Vec<&str> = rest.splitn(5, ' ').map(str::trim).filter(|s| !s.is_empty()).collect();
            if w.len() < 4 {
                println!("[错误] 用法：model add <id> <展示名> <实际模型串> <供应商id> [note]");
                return;
            }
            let note = w.get(4).copied().unwrap_or("");
            match core.model_upsert(w[0], w[1], w[2], w[3], note) {
                Ok(()) => println!("[登记] 模型 {}", w[0]),
                Err(e) => println!("[错误] {}", e),
            }
        }
        "rm" => match core.model_remove(rest) {
            Ok(true) => println!("[移除] 模型 {}", rest),
            Ok(false) => println!("[错误] 无此模型：{}", rest),
            Err(e) => println!("[错误] {}", e),
        },
        _ => println!("[错误] 用法：model list|add|rm …"),
    }
}

fn core_flow(core: &mut Core, arg: &str) {
    if arg.is_empty() {
        println!("核心默认模型：{}", core.core_model().unwrap_or_else(|| "（未设定）".to_string()));
        return;
    }
    match core.core_set_model(arg) {
        Ok(true) => println!("[核心默认] {}", arg),
        Ok(false) => println!("[错误] 无此模型：{}", arg),
        Err(e) => println!("[错误] {}", e),
    }
}

fn prompt(text: &str) -> String {
    print!("{} ", text);
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).ok();
    s.trim().to_string()
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
