//! Solomni 核心宿主：扫描 modules/、呈现菜单、路由三种模式。
//! 转录中心原则：呈现给用户的内容与进入上下文的完全一致。

mod envelope;
mod model;
mod module;
mod orchestrator;
mod providers;
#[cfg(test)]
mod tests;

use model::{Chat, FakeChat, Msg};
use std::path::PathBuf;

fn main() {
    let root = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let modules_dir = root.join("modules");
    println!("Solomni 核心骨架（转录中心 v0）");

    // 供应商登记处：核心私有区（.home/），密钥唯一合法居所。
    let registry = providers::Registry::load(&root.join(".home/providers.yaml"));

    // 清单即事实：每次启动重新扫描。
    let roster = module::scan(&modules_dir);
    println!("[发现] {}", if roster.modules.is_empty() { "（无模块）".into() } else {
        roster.modules.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>().join(" · ")
    });
    for r in &roster.rejected {
        println!("[拒收] {}", r);
    }

    // 模型可用性：无通道时只提供假模型演示（如实告知，不静默）。
    let has_channel = registry.default.is_some() || roster.modules.iter().any(|m| m.selected_provider.is_some());
    if !has_channel {
        println!("[提示] 未配置任何供应商：本会话使用内置假模型演示流程（假模型 · 无网络）");
    }

    loop {
        println!("\n可唤起模块：");
        for m in &roster.modules {
            println!("  {:<14} {}", m.manifest.id, first_line(&m.manifest.brief));
        }
        println!("命令：direct <id> | collab <id>[,<id>…] | omni | exit");
        print!("> ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim().to_string();
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();
        match cmd {
            "direct" if !arg.is_empty() => direct_mode(&roster, arg),
            "collab" if !arg.is_empty() => collab_mode(&roster, arg, &registry),
            "omni" => omni_mode(&roster),
            "exit" => break,
            _ => continue,
        }
    }
    println!("再见。");
}

/// 模式一：单模块直连（本骨架版走假模型演示上下文拼装）。
fn direct_mode(roster: &module::Roster, id: &str) {
    let Some(m) = roster.modules.iter().find(|m| m.manifest.id == id) else {
        println!("[错误] 无此模块：{}", id);
        return;
    };
    println!("（直连 {} —— 上下文 = system + 用户消息。骨架版：输入一行，演示一轮）", id);
    print!("你> ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut say = String::new();
    std::io::stdin().read_line(&mut say).ok();
    let mut chat = demo_chat_for(m);
    let reply = chat.complete(&[Msg::system(m.system_block()), Msg::user(say.trim())]);
    println!("{}> {}", id, model::last_text(&reply));
}

/// 模式二：多模块协作（建组 → 讨论 → 整理 → 执行 → 验收）。
fn collab_mode(roster: &module::Roster, ids: &str, _registry: &providers::Registry) {
    let picked: Vec<&module::Module> = ids
        .split(|c| c == ',' || c == '，')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id))
        .collect();
    if picked.is_empty() {
        println!("[错误] 名单为空或无有效模块");
        return;
    }
    println!("[建组] {}", picked.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>().join(" + "));

    print!("需求> ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut task = String::new();
    std::io::stdin().read_line(&mut task).ok();
    let task = task.trim().to_string();

    // 本骨架用假模型演示五阶段流转（真实通道接入后按登记处替换）。
    let scripts: Vec<Vec<String>> = picked
        .iter()
        .map(|m| vec![
            model::FakeChat::say(&format!("{}：我先看需求，略陈管见。", m.manifest.id)),
            model::FakeChat::verb_json("agree", "同意当前方案"),
        ])
        .collect();
    let mut chats: Vec<FakeChat> = scripts.into_iter().map(FakeChat::new).collect();
    let mut core_chat = FakeChat::new(vec![
        "== 任务清单 ==\n1. 各成员按分工执行".to_string(),
        "== 验收清单 ==\n1. pass — 回报与方案一致".to_string(),
    ]);

    let members: Vec<orchestrator::Member> = picked
        .iter()
        .zip(chats.iter_mut())
        .map(|(m, c)| orchestrator::Member { id: &m.manifest.id, system: m.system_block(), chat: c, present: true, agreed: false })
        .collect();

    let mut disc = orchestrator::Discussion { members, transcript: Vec::new(), round: 0, pending_user_answers: Vec::new(), closed: false };
    let protocol = "讨论约定：直接说事；需要用户决定时用 ask；不再参与用 leave；同意方案用 agree。";
    disc.open(&task, protocol);
    print_transcript(&disc.transcript);

    loop {
        match disc.step() {
            orchestrator::TurnOut::Round => print_transcript(&disc.transcript),
            orchestrator::TurnOut::AskUser { member, question } => {
                println!("[请教] {}：{}", member, question);
                print!("你的回答（回车继续）> ");
                use std::io::Write;
                std::io::stdout().flush().ok();
                let mut ans = String::new();
                std::io::stdin().read_line(&mut ans).ok();
                disc.pending_user_answers.push(ans.trim().to_string());
            }
            orchestrator::TurnOut::Done => {
                if disc.round > orchestrator::MAX_ROUNDS {
                    println!("[上限] 讨论轮次超限，交用户裁决。");
                }
                break;
            }
        }
    }

    let plan = disc.synthesize(&mut core_chat);
    println!("[整理] \n{}", plan);

    let mut exec = orchestrator::Execution::run(&mut disc.members, &plan);
    println!("[执行] ");
    for (id, report) in &exec.reports {
        println!("[{}] {}", id, report);
    }
    exec.review(&mut core_chat, &plan);
    println!("[验收] \n{}", exec.checklist);
    if exec.all_pass() {
        println!("[交付] 全部通过，交付用户。");
    } else {
        println!("[返工] 存在未通过项（骨架版不循环返工）。");
    }
}

/// 模式三：全能模式（拼装所有模块 system 为一份提示词）。
fn omni_mode(roster: &module::Roster) {
    let mut merged = String::from("你是全能助手，能力由以下模块职责拼装：\n");
    for m in &roster.modules {
        merged.push_str(&format!("\n== {} ==\n{}", m.manifest.id, m.manifest.system));
    }
    println!("[全能] 已拼装 {} 个模块的职责提示词。骨架版：输入一行，演示一轮。", roster.modules.len());
    print!("你> ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut say = String::new();
    std::io::stdin().read_line(&mut say).ok();
    let mut chat = FakeChat::new(vec![FakeChat::say("（全能演示）收到。")]);
    let _ = &merged;
    let reply = chat.complete(&[Msg::system(merged), Msg::user(say.trim())]);
    println!("全能> {}", model::last_text(&reply));
}

fn demo_chat_for(m: &module::Module) -> Box<dyn Chat> {
    let _ = m;
    Box::new(FakeChat::new(vec![FakeChat::say("（直连演示）收到。")]))
}

fn print_transcript(t: &[String]) {
    println!("---- 转录 ----");
    for line in t {
        println!("{}", line);
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
