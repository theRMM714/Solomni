//! 终端转录中心：解析命令 → 调核心门面 → 渲染事件流。
//! 只做解析与渲染，不做业务决策；Web 前端与它并列、共用同一门面与事件词汇。

use crate::core::{CollabStep, Core, Pending, SessionEvent, SessionId};
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
        let cmd = parts.next().unwrap_or("").to_string();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.to_ascii_lowercase().as_str() {
            "direct" if !arg.is_empty() => direct_flow(&mut core, &arg),
            "collab" if !arg.is_empty() => match core.start_collab(&arg) {
                Ok(sid) => collab_flow(&mut core, sid),
                Err(e) => println!("[错误] {}", e),
            },
            "omni" => match core.start_omni(&arg) {
                Ok((sid, open)) => omni_flow(&mut core, sid, open),
                Err(e) => println!("[错误] {}", e),
            },
            "provider" => provider_flow(&mut core, &arg),
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
    println!(
        "[发现] {}",
        if roster.modules.is_empty() {
            "（无模块）".to_string()
        } else {
            roster.modules.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>().join(" · ")
        }
    );
    for r in &roster.rejected {
        println!("[拒收] {}", r);
    }
}

fn print_menu(core: &Core) {
    println!("\n可唤起模块：");
    for m in &core.scan().modules {
        println!("  {:<14} {}", m.manifest.id, first_line(&m.manifest.brief));
    }
    println!("命令：direct <id> | collab <id>[,<id>…] | collab ?（代拟） | omni [id…] | provider list|add|key|rm|default | rescan | webui | exit");
}

// ---------- 事件渲染：CLI 与 Web 前端同源 ----------

fn render(events: &[SessionEvent]) {
    for e in events {
        match e {
            SessionEvent::Notice(n) => println!("{}", n),
            SessionEvent::Transcript(lines) => {
                println!("---- 转录 ----");
                for l in lines {
                    println!("{}", l);
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
        }
    }
}

// ---------- 模式一：直连 ----------

fn direct_flow(core: &mut Core, id: &str) {
    let (sid, open) = match core.start_direct(id) {
        Ok(x) => x,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    render(&open);
    println!("（直连 {} —— 输入消息，空行结束会话）", id);
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        match core.direct_say(sid, &say) {
            Ok(events) => render(&events),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

// ---------- 模式三：全能 ----------

fn omni_flow(core: &mut Core, sid: SessionId, open: Vec<SessionEvent>) {
    render(&open);
    println!("[全能] 会话开始（空行结束）。");
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        match core.omni_say(sid, &say) {
            Ok(events) => render(&events),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

// ---------- 模式二：协作（按核心 pending 驱动） ----------

fn collab_flow(core: &mut Core, sid: SessionId) {
    let task = prompt("需求>");
    match core.collab_continue(sid, CollabStep::SetTask, &task) {
        Ok(events) => render(&events),
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    }

    // 名单确认（代拟路径）。
    if matches!(core.collab_pending(sid), Ok(Some(Pending::ConfirmSlate))) {
        let ok = prompt("确认名单？（yes 开始 / 其他取消）");
        match core.collab_continue(sid, CollabStep::ConfirmSlate, &ok) {
            Ok(events) => render(&events),
            Err(e) => println!("[错误] {}", e),
        }
    }
    // 开始确认。
    if matches!(core.collab_pending(sid), Ok(Some(Pending::ConfirmBegin))) {
        let ans = prompt("开始讨论？（yes / yes,allow：授权小组自裁细节）");
        match core.collab_continue(sid, CollabStep::Begin, &ans) {
            Ok(events) => render(&events),
            Err(e) => println!("[错误] {}", e),
        }
    }
    // ask 循环（每次回答后可能接新的请教）。
    while matches!(core.collab_pending(sid), Ok(Some(Pending::Ask { .. }))) {
        if let Ok(Some(Pending::Ask { member, question })) = core.collab_pending(sid) {
            println!("[请教] {}：{}", member, question);
            let ans = prompt("你的回答（回车 = 无补充，继续）>");
            match core.collab_continue(sid, CollabStep::Answer, &ans) {
                Ok(events) => render(&events),
                Err(e) => {
                    println!("[错误] {}", e);
                    break;
                }
            }
        }
    }
}

// ---------- 供应商管理（密钥只在核心层进出登记处） ----------

fn provider_flow(core: &mut Core, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let sub = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match sub {
        "list" => {
            let lines = core.provider_list();
            if lines.is_empty() {
                println!("（登记处为空）用 provider add <id> <base_url> <api_key> [模型,…] 添加");
            } else if let Some(d) = core.provider_default() {
                println!("（当前默认：{}）", d);
            }
            for l in lines {
                println!("  {}", l);
            }
        }
        "add" | "key" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            if words.len() < 3 {
                println!("[错误] 用法：provider add <id> <base_url> <api_key> [模型,…]");
                return;
            }
            let (id, url, key) = (words[0], words[1], words[2]);
            let models: Vec<String> = words[3..].iter().map(|s| s.to_string()).collect();
            match core.provider_upsert(id, url, key, &models) {
                Ok(()) => println!("[登记] {} 已保存（0600）", id),
                Err(e) => println!("[错误] {}", e),
            }
        }
        "rm" => match core.provider_remove(rest) {
            Ok(true) => println!("[移除] {}", rest),
            Ok(false) => println!("[错误] 无此供应商：{}", rest),
            Err(e) => println!("[错误] {}", e),
        },
        "default" => match core.provider_set_default(rest) {
            Ok(true) => println!("[默认] {}", rest),
            Ok(false) => println!("[错误] 无此供应商：{}", rest),
            Err(e) => println!("[错误] {}", e),
        },
        _ => println!("[错误] 用法：provider list|add|key|rm|default …"),
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
