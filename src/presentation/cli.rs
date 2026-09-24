//! 终端转录中心：解析命令 → 用入站能力面 → 渲染事件流。
//! 只做解析与渲染，不做业务决策；Web 前端与它并列，共用同一能力面与事件词汇。

use crate::core::api::{Ops, Output};
use crate::core::providers::{ModelView, ProviderView};
use crate::core::{AgentInstance, CollabStep, Pending, SessionEvent, WorkMode};
use crate::presentation::intent;
use crate::presentation::web::DEFAULT_PORT;
use std::io::Write;

/// 离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

pub fn run(ops: Ops) -> CliExit {
    println!("Solomni 核心编排者（转录中心）");
    print_roster(&ops);

    loop {
        print_menu(&ops);
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
            "single" => single_flow(&ops, &arg),
            "collab" => collab_flow(&ops, &arg),
            "provider" => provider_flow(&ops, &arg),
            "model" => model_flow(&ops, &arg),
            "core" => core_flow(&ops, &arg),
            "rescan" => print_roster(&ops),
            // 转入 Web 转录中心：接受 webui / -webUI（启动参数也这么写），可选端口。
            "webui" | "-webui" | "web" | "-web" => {
                let port = arg.parse::<u16>().unwrap_or(DEFAULT_PORT);
                return CliExit::Web(port);
            }
            "exit" => break,
            "" => continue,
            _ => println!("[提示] 未知命令 {}（Web 界面用 webui；退出用 exit）", cmd),
        }
    }
    println!("再见。");
    CliExit::Exit
}

fn print_roster(ops: &Ops) {
    let roster = match ops.discovery.roster() {
        Ok(r) => r,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let tier = match ops.registry.settings() {
        Ok(s) => s.tier,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    // 运行能力报告与模块清单同源：按默认执行档位如实报（缺包不是崩溃，工具按档位不可用）。
    let report = match ops.discovery.runtime_report(tier) {
        Ok(r) => r,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    println!(
        "[发现] {}",
        if roster.modules.is_empty() {
            "（无模块）".to_string()
        } else {
            roster
                .modules
                .iter()
                .map(|m| m.manifest.id.clone())
                .collect::<Vec<_>>()
                .join(" · ")
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
        println!(
            "[缺运行包] 模块 {} 需要 {}；把包放进依赖文件夹 runtimes/（契约见 RUNTIME_SPEC.md）",
            id,
            caps.join("、")
        );
    }
    // 虚拟机档的诊断：缺包之外（多版本未定版 / 定版不存在 / 路径冲突）会挡住「开始」，在这里如实说明。
    let hard: Vec<crate::core::exec::Diagnosis> = report
        .diagnoses
        .iter()
        .filter(|d| !matches!(d, crate::core::exec::Diagnosis::Missing { .. }))
        .cloned()
        .collect();
    if !hard.is_empty() {
        println!(
            "[档位诊断] {}（虚拟机档要先解决这些才能开始会话）",
            crate::core::exec::diagnose_text(&hard)
        );
    }
    for r in &report.rejected_packages {
        println!("[运行包拒收] {}", r);
    }
}

fn print_menu(ops: &Ops) {
    let agents = match ops.registry.agents() {
        Ok(a) => a,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    println!("\n可唤起 agent（发言席只有 agent；模块是它的能力包）：");
    if agents.is_empty() {
        println!("  （登记处还没有 agent：到 Web 界面「设置 → agent 管理」建一个）");
    }
    for a in &agents {
        println!(
            "  {:<14} {} · 模型 {}",
            a.name,
            a.modules.join(" + "),
            model_label(a.model.as_deref())
        );
    }
    println!("命令：single [agent名…] | collab [agent名…|?] | provider list|add|rm|discover | model list|add|rm | core <模型id> | rescan | webui | exit");
}

/// 模型标签（CLI 展示文案；核心默认是登记处的概念，不是提示词）。
fn model_label(model: Option<&str>) -> String {
    model
        .map(|m| m.to_string())
        .unwrap_or_else(|| "（核心默认）".to_string())
}

/// 展示文案归呈现层：core 只给结构化事实（视图），怎么排版是这里的事。
fn provider_line(p: &ProviderView) -> String {
    format!("{}  {}", p.id, p.base_url)
}

fn model_line(m: &ModelView, core: Option<&str>) -> String {
    let mark = if core == Some(m.id.as_str()) {
        "（核心默认）"
    } else {
        ""
    };
    format!(
        "{}{}  {} → {}  [{}]",
        m.id, mark, m.name, m.api_model, m.provider
    )
}

// ---------- 事件渲染：CLI 与 Web 前端同源 ----------

fn render(events: &[SessionEvent]) {
    for e in events {
        match e {
            SessionEvent::Notice(n) => println!("{}", n),
            SessionEvent::Transcript(lines) => {
                println!("---- 转录 ----");
                for l in lines {
                    // 系统消息（提醒/"未回应"这类**不是谁说的**内容）标出来，别和用户/发言混在一起。
                    let mark = if l.system { "[系统] " } else { "" };
                    println!("{}{}", mark, l.line);
                }
            }
            SessionEvent::DiscussionDone { .. } => {}
            SessionEvent::Compacted { up_to, summary } => {
                println!(
                    "[压缩] 此前内容已压成摘要（不再发给模型，仍可查看）：\n{}",
                    summary
                );
                println!("  （压缩到第 {} 行）", up_to);
            }
            SessionEvent::NodeStarted {
                node,
                sid,
                assignee,
            } => {
                println!(
                    "[派发] 节点 {} → 子会话 {}（负责人 {}）",
                    node, sid, assignee
                )
            }
            SessionEvent::Plan(p) => println!("[整理] \n{}", p),
            SessionEvent::PlanReview { plan, chain } => {
                println!("[待审] 方案如下，点「同意」才开工：\n{}", plan);
                for n in &chain.nodes {
                    println!(
                        "  - {}（{}）负责人 {} 依赖 {:?}",
                        n.id, n.title, n.assignee, n.deps
                    );
                }
            }
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

// ---------- 形态一：单 agent（模块数不限） ----------

fn single_flow(ops: &Ops, arg: &str) {
    let views = match intent::all_views(ops) {
        Ok(v) => v,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    if views.is_empty() {
        println!("[错误] {}（CLI 不再直接点模块）", intent::NO_AGENTS);
        return;
    }
    // 点名 1 个 = 直接用该 agent（模块数不限）；点名多个 = 把那几个的模块并成一个；无参 = 把登记处全部并成一个。
    let picked = if arg.trim().is_empty() {
        intent::as_instances(&views)
    } else {
        match intent::pick_agents(ops, &intent::split_names(arg)) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let agent = if picked.len() == 1 {
        picked.into_iter().next().expect("刚判过长度")
    } else {
        intent::merge_into_one(&picked, "组合")
    };
    let work_name = match intent::unique_work_name(ops, "single", "single") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let opened = match intent::open_work(ops, work_name, WorkMode::Single, vec![agent], None, false)
    {
        Ok(o) => o,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    render(&opened.events);
    let sid = opened.sid;
    println!("（单 agent {} —— 输入消息，空行结束会话）", sid);
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        // 终端只在最终结果上渲染，不要流式（怎么显示是呈现层的事）。
        match intent::act(ops, &sid, intent::Action::Say(&say), Output::Final) {
            Ok(acted) => render(&intent::into_events(acted)),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

// ---------- 模式三：协作（按核心 pending 驱动） ----------

fn collab_flow(ops: &Ops, arg: &str) {
    let trimmed = arg.trim();
    let delegate = trimmed.is_empty() || trimmed == "?";
    let task = prompt("需求>");
    // 代拟（无参或 ?）= 核心拟名单；点名 = 用登记处里的那几个 agent。
    let agents: Vec<AgentInstance> = if delegate {
        Vec::new()
    } else {
        match intent::pick_agents(ops, &intent::split_names(trimmed)) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let work_name = match intent::unique_work_name(ops, "collab", "collab") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let sid = match intent::open_work(
        ops,
        work_name,
        WorkMode::Collab,
        agents,
        Some(task),
        delegate,
    ) {
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
    if matches!(ops.sessions.pending(&sid), Ok(Some(Pending::ConfirmSlate))) {
        match ops.sessions.slate(&sid) {
            Ok(list) => {
                println!("[代拟] 核心拟的名单：");
                for a in list {
                    println!(
                        "  {}：模块 {} · 模型 {} · {}",
                        a.name,
                        a.modules.join(" + "),
                        model_label(a.model.as_deref()),
                        if a.transient {
                            "组装（临时）"
                        } else {
                            "复用已存 agent"
                        }
                    );
                }
            }
            Err(e) => println!("[提示] 取名单失败：{}", e),
        }
        let ok = prompt("确认名单？（yes 开始 / 其他取消）");
        match intent::act(
            ops,
            &sid,
            intent::Action::Step(CollabStep::ConfirmSlate, &ok),
            Output::Final,
        ) {
            Ok(acted) => render(&intent::into_events(acted)),
            Err(e) => println!("[错误] {}", e),
        }
    }
    // 开始确认。
    if matches!(ops.sessions.pending(&sid), Ok(Some(Pending::ConfirmBegin))) {
        let ans = prompt("开始讨论？（yes / yes,allow：授权小组自裁细节）");
        match intent::act(
            ops,
            &sid,
            intent::Action::Step(CollabStep::Begin, &ans),
            Output::Final,
        ) {
            Ok(acted) => render(&intent::into_events(acted)),
            Err(e) => println!("[错误] {}", e),
        }
    }
    // ask 循环（每次回答后可能接新的请教）。
    while matches!(ops.sessions.pending(&sid), Ok(Some(Pending::Ask { .. }))) {
        if let Ok(Some(Pending::Ask { member, question })) = ops.sessions.pending(&sid) {
            println!("[请教] {}：{}", member, question);
            let ans = prompt("你的回答（回车 = 无补充，继续）>");
            match intent::act(
                ops,
                &sid,
                intent::Action::Step(CollabStep::Answer, &ans),
                Output::Final,
            ) {
                Ok(acted) => render(&intent::into_events(acted)),
                Err(e) => {
                    println!("[错误] {}", e);
                    break;
                }
            }
        }
    }
}

// ---------- 登记处管理（密钥只在核心层进出） ----------

fn provider_flow(ops: &Ops, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let sub = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match sub {
        "list" | "" => {
            let views = match ops.registry.providers() {
                Ok(v) => v,
                Err(e) => {
                    println!("[错误] {}", e);
                    return;
                }
            };
            if views.is_empty() {
                println!("（无供应商）用 provider add <id> <base_url> <api_key> 添加");
            }
            for p in &views {
                println!("  {}", provider_line(p));
            }
        }
        "add" | "key" => {
            let w: Vec<&str> = rest.split_whitespace().collect();
            if w.len() < 3 {
                println!("[错误] 用法：provider add <id> <base_url> <api_key>");
                return;
            }
            match ops.registry.upsert_provider(w[0], w[1], w[2]) {
                Ok(()) => println!("[登记] {} 已保存（0600）", w[0]),
                Err(e) => println!("[错误] {}", e),
            }
        }
        "rm" => match ops.registry.remove_provider(rest) {
            Ok(true) => println!("[移除] {}", rest),
            Ok(false) => println!("[错误] 无此供应商：{}", rest),
            Err(e) => println!("[错误] {}", e),
        },
        "discover" => match ops.registry.discover_models(rest) {
            Ok(models) => println!("[发现] {}：{}", rest, models.join(" · ")),
            Err(e) => println!("[错误] {}", e),
        },
        _ => println!("[错误] 用法：provider list|add|rm|discover …"),
    }
}

fn model_flow(ops: &Ops, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let sub = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match sub {
        "list" | "" => {
            let views = match ops.registry.models() {
                Ok(v) => v,
                Err(e) => {
                    println!("[错误] {}", e);
                    return;
                }
            };
            let core = ops.registry.core_model().ok().flatten();
            if views.is_empty() {
                println!(
                    "（无模型）用 model add <id> <展示名> <实际模型串> <供应商id> [note] 添加"
                );
            }
            for m in &views {
                println!("  {}", model_line(m, core.as_deref()));
            }
        }
        "add" => {
            let w: Vec<&str> = rest
                .splitn(5, ' ')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if w.len() < 4 {
                println!("[错误] 用法：model add <id> <展示名> <实际模型串> <供应商id> [note]");
                return;
            }
            let note = w.get(4).copied().unwrap_or("");
            match ops.registry.upsert_model(w[0], w[1], w[2], w[3], note, 0) {
                Ok(()) => println!("[登记] 模型 {}", w[0]),
                Err(e) => println!("[错误] {}", e),
            }
        }
        "rm" => match ops.registry.remove_model(rest) {
            Ok(true) => println!("[移除] 模型 {}", rest),
            Ok(false) => println!("[错误] 无此模型：{}", rest),
            Err(e) => println!("[错误] {}", e),
        },
        _ => println!("[错误] 用法：model list|add|rm …"),
    }
}

fn core_flow(ops: &Ops, arg: &str) {
    if arg.is_empty() {
        let cur = ops.registry.core_model().ok().flatten();
        println!(
            "核心默认模型：{}",
            cur.unwrap_or_else(|| "（未设定）".to_string())
        );
        return;
    }
    match ops.registry.set_core_model(arg) {
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
