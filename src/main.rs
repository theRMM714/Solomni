//! Solomni 核心宿主：扫描 modules/、呈现菜单、路由三种模式。
//! 转录中心原则：呈现给用户的内容与进入上下文的完全一致。

mod envelope;
mod model;
mod module;
mod orchestrator;
mod providers;
#[cfg(test)]
mod tests;

use model::{Chat, FakeChat, HttpChat, Msg};
use std::path::PathBuf;

/// 会话通道：真实（按登记处解析）或演示（假模型）。
enum Channel<'a> {
    Real(HttpChat<'a>),
    Demo(FakeChat),
}

impl<'a> Chat for Channel<'a> {
    fn complete(&mut self, messages: &[Msg]) -> String {
        match self {
            Channel::Real(c) => c.complete(messages),
            Channel::Demo(c) => c.complete(messages),
        }
    }
}

/// 一条已就绪的成员：id + system + 通道。
struct Ready<'a> {
    id: &'a str,
    system: String,
    chat: Box<dyn Chat + 'a>,
}

fn main() {
    let root = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let modules_dir = root.join("modules");
    println!("Solomni 核心编排者（转录中心）");

    // 供应商登记处：核心私有区（.home/），密钥唯一合法居所。
    let registry_path = root.join(".home").join("providers.yaml");
    let mut registry = providers::Registry::load(&registry_path);

    // 清单即事实：每次启动、每次建组都重新扫描。
    let mut roster = module::scan(&modules_dir);
    print_roster(&roster);

    loop {
        println!("\n可唤起模块：");
        for m in &roster.modules {
            println!("  {:<14} {}", m.manifest.id, first_line(&m.manifest.brief));
        }
        println!(
            "命令：direct <id> | collab <id>[,<id>…] | omni [id…] | provider list|add|key|rm|default | rescan | exit"
        );
        print!("> ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim().to_string();
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").to_string();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "direct" if !arg.is_empty() => {
                roster = module::scan(&modules_dir);
                let registry = providers::Registry::load(&registry_path);
                direct_mode(&roster, &arg, &registry);
            }
            "collab" if !arg.is_empty() => {
                roster = module::scan(&modules_dir);
                let registry = providers::Registry::load(&registry_path);
                collab_mode(&roster, &arg, &registry);
            }
            "omni" => {
                roster = module::scan(&modules_dir);
                let registry = providers::Registry::load(&registry_path);
                omni_mode(&roster, &arg, &registry);
            }
            "provider" => provider_cmd(&mut registry, &registry_path, &arg),
            "rescan" => {
                roster = module::scan(&modules_dir);
                print_roster(&roster);
            }
            "exit" => break,
            _ => continue,
        }
    }
    println!("再见。");
}

fn print_roster(roster: &module::Roster) {
    println!("[发现] {}", if roster.modules.is_empty() { "（无模块）".to_string() } else {
        roster.modules.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>().join(" · ")
    });
    for r in &roster.rejected {
        println!("[拒收] {}", r);
    }
}

// ---------- 供应商管理（用户经产品命令管理；密钥只进登记处） ----------

fn provider_cmd(registry: &mut providers::Registry, path: &std::path::Path, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let sub = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match sub {
        "list" => {
            let lines = registry.display_lines();
            if lines.is_empty() {
                println!("（登记处为空）用 provider add <id> <base_url> <api_key> [模型,…] 添加");
            }
            for l in lines {
                println!("  {}", l);
            }
            if let Some(d) = &registry.default {
                println!("  全局默认：{}", d);
            }
        }
        "add" | "key" => {
            let words: Vec<&str> = rest.split_whitespace().collect();
            if words.len() < 3 {
                println!("[错误] 用法：provider add <id> <base_url> <api_key> [模型,…]");
                return;
            }
            let (id, url, key) = (words[0], words[1], words[2]);
            let models = words[3..].join(",");
            if id.is_empty() || url.is_empty() || key.is_empty() {
                println!("[错误] id / base_url / api_key 均不能为空");
                return;
            }
            let entry = providers::Provider {
                kind: "llm".to_string(),
                base_url: url.to_string(),
                api_key: key.to_string(),
                models: models.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            };
            registry.providers.insert(id.to_string(), entry);
            if registry.default.is_none() {
                registry.default = Some(id.to_string());
            }
            match registry.save(path) {
                Ok(()) => println!("[登记] {} 已保存（0600）", id),
                Err(e) => println!("[错误] 保存失败：{}", e),
            }
        }
        "rm" => {
            if registry.providers.remove(rest).is_some() {
                if registry.default.as_deref() == Some(rest) {
                    registry.default = registry.providers.keys().next().cloned();
                }
                match registry.save(path) {
                    Ok(()) => println!("[移除] {}", rest),
                    Err(e) => println!("[错误] 保存失败：{}", e),
                }
            } else {
                println!("[错误] 无此供应商：{}", rest);
            }
        }
        "default" => {
            if registry.providers.contains_key(rest) {
                registry.default = Some(rest.to_string());
                match registry.save(path) {
                    Ok(()) => println!("[默认] {}", rest),
                    Err(e) => println!("[错误] 保存失败：{}", e),
                }
            } else {
                println!("[错误] 无此供应商：{}", rest);
            }
        }
        _ => println!("[错误] 用法：provider list|add|key|rm|default …"),
    }
}

// ---------- 通道装配：模块当前选择 > 清单默认 > 全局默认；回落如实告知 ----------

fn make_chat<'a>(
    registry: &'a providers::Registry,
    module: &'a module::Module,
) -> (Channel<'a>, Option<String>) {
    match registry.resolve(module.selected_provider.as_deref(), module.manifest.model.provider.as_deref()) {
        Some((_id, provider)) => {
            let model = provider
                .models
                .first()
                .cloned()
                .unwrap_or_else(|| "default".to_string());
            (Channel::Real(HttpChat { provider, model }), None)
        }
        None => (
            Channel::Demo(FakeChat::new(vec![FakeChat::say("（演示）收到。")])),
            Some(format!("{} 未配置供应商，使用内置假模型演示", module.manifest.id)),
        ),
    }
}

fn core_chat<'a>(registry: &'a providers::Registry) -> Channel<'a> {
    match registry.resolve(None, None) {
        Some((_, p)) => Channel::Real(HttpChat {
            provider: p,
            model: p.models.first().cloned().unwrap_or_else(|| "default".to_string()),
        }),
        None => Channel::Demo(FakeChat::new(vec![])),
    }
}

fn prompt(text: &str) -> String {
    print!("{} ", text);
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).ok();
    s.trim().to_string()
}

// ---------- 模式一：单模块直连 ----------

fn direct_mode(roster: &module::Roster, id: &str, registry: &providers::Registry) {
    let Some(m) = roster.modules.iter().find(|m| m.manifest.id == id) else {
        println!("[错误] 无此模块：{}", id);
        return;
    };
    let (mut chat, note) = make_chat(registry, m);
    if let Some(n) = note {
        println!("[提示] {}", n);
    }
    println!("（直连 {} —— 输入消息，空行结束会话）", id);
    let mut history: Vec<Msg> = vec![Msg::system(m.system_block())];
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        history.push(Msg::user(say));
        let raw = chat.complete(&history);
        let reply = envelope::parse(&raw);
        println!("{}> {}", id, reply.text);
        history.push(Msg::assistant(reply.text));
    }
}

// ---------- 模式二：多模块协作 ----------

fn collab_mode(roster: &module::Roster, ids: &str, registry: &providers::Registry) {
    // 建组：只认用户点名的名单（代拟确认流程见 plan_slate）。
    let picked = match plan_slate(roster, ids, registry) {
        Some(p) => p,
        None => return,
    };
    println!("[建组] {}", picked.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>().join(" + "));
    let task = prompt("需求>");
    if task.is_empty() {
        println!("[取消] 需求为空");
        return;
    }
    let answer = prompt("开始讨论？（yes / yes,allow：授权小组自裁细节 / 改名单请重新 collab）");
    let allow = answer.contains("allow");

    // 装配通道（每个成员独立会话；回落如实告知）。
    let mut ready: Vec<Ready> = Vec::new();
    for m in &picked {
        let (chat, note) = make_chat(registry, m);
        if let Some(n) = note {
            println!("[提示] {}", n);
        }
        ready.push(Ready { id: &m.manifest.id, system: m.system_block(), chat: Box::new(chat) });
    }
    let members: Vec<orchestrator::Member> = ready
        .iter_mut()
        .map(|r| orchestrator::Member { id: r.id, system: r.system.clone(), chat: r.chat.as_mut(), present: true, agreed: false })
        .collect();

    let mut disc = orchestrator::Discussion {
        members,
        transcript: Vec::new(),
        round: 0,
        pending_user_answers: Vec::new(),
        closed: false,
        allow_autonomy: allow,
    };
    let protocol = "讨论约定：直接说事；需要用户决定时用 ask；不再参与用 leave；同意方案用 agree。";
    disc.open(&task, protocol);
    print_transcript(&disc.transcript);

    loop {
        match disc.step() {
            orchestrator::TurnOut::Round => print_transcript(&disc.transcript),
            orchestrator::TurnOut::AskUser { member, question } => {
                println!("[请教] {}：{}", member, question);
                let ans = prompt("你的回答（回车 = 无补充，继续）>");
                disc.pending_user_answers.push(ans);
            }
            orchestrator::TurnOut::Done => {
                if disc.round > orchestrator::MAX_ROUNDS {
                    println!("[上限] 讨论轮次超限，交用户裁决。");
                }
                break;
            }
        }
    }

    // 整理：核心接入。
    let mut core = core_chat(registry);
    let plan = disc.synthesize(&mut core);
    println!("[整理] \n{}", plan);

    // 执行 → 验收 → 返工（上限内）→ 交付。
    let mut exec = orchestrator::Execution::run(disc.members.as_mut_slice(), &plan);
    println!("[执行]");
    for (id, report) in &exec.reports {
        println!("[{}] {}", id, report);
    }
    exec.review(&mut core, &plan);
    print_review(&exec);
    while !exec.all_pass() && exec.rework < orchestrator::MAX_REWORK {
        println!("[返工] 第 {} 次（上限 {}）", exec.rework + 1, orchestrator::MAX_REWORK);
        let review_text = exec
            .items
            .iter()
            .filter(|i| !i.status.eq_ignore_ascii_case("pass"))
            .map(|i| format!("- {}：{}", i.item, i.reason.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n");
        exec.rerun(disc.members.as_mut_slice(), &plan, &review_text);
        println!("[执行] （返工后）");
        for (id, report) in &exec.reports {
            println!("[{}] {}", id, report);
        }
        exec.review(&mut core, &plan);
        print_review(&exec);
    }
    if exec.all_pass() {
        println!("[交付] 全部通过，交付用户。");
    } else {
        println!("[裁决] 返工超限仍未通过，交用户裁决。");
    }
}

/// 建组名单：用户点名，或「?」委托核心代拟（附理由，经确认才成立）。
fn plan_slate<'a>(
    roster: &'a module::Roster,
    ids: &str,
    registry: &providers::Registry,
) -> Option<Vec<&'a module::Module>> {
    if ids.trim() != "?" {
        let picked: Vec<&module::Module> = ids
            .split(|c| c == ',' || c == '，')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id))
            .collect();
        if picked.is_empty() {
            println!("[错误] 名单为空或无有效模块");
            return None;
        }
        return Some(picked);
    }
    // 委托代劳：核心代拟名单（LLM 有通道用 LLM；否则如实说明并退出）。
    let mut core = core_chat(registry);
    let listing = roster
        .modules
        .iter()
        .map(|m| format!("- {}：{}", m.manifest.id, m.manifest.brief))
        .collect::<Vec<_>>()
        .join("\n");
    let raw = core.complete(&[
        Msg::system("你是核心编排者。根据需求从模块简述中代拟建组名单。只输出 JSON：{\"picks\":[{\"id\":\"模块id\",\"why\":\"一句入选理由\"}]}。不选择不存在的模块。"),
        Msg::user(&format!("== 模块简述 ==\n{}\n\n== 需求 ==\n{}", listing, prompt("需求（供代拟参考）>"))),
    ]);
    let Some(arr_json) = crate::envelope::extract_json_object(&raw) else {
        println!("[错误] 代拟失败（模型无响应格式）。请直接点名模块。");
        return None;
    };
    #[derive(serde::Deserialize)]
    struct Pick { id: String, why: String }
    #[derive(serde::Deserialize)]
    struct Slate { picks: Vec<Pick> }
    let Ok(slate) = serde_json::from_str::<Slate>(&arr_json) else {
        println!("[错误] 代拟格式非法。请直接点名模块。");
        return None;
    };
    // 呈现代拟名单与理由，逐个核对存在性；非法 id 拒收（校验，不是挑选）。
    println!("[代拟] 核心建议：");
    let mut confirmed: Vec<&module::Module> = Vec::new();
    for p in &slate.picks {
        match roster.modules.iter().find(|m| m.manifest.id == p.id) {
            Some(m) => {
                println!("  + {}（{}）", p.id, p.why);
                confirmed.push(m);
            }
            None => println!("  - {}（不存在，拒收）", p.id),
        }
    }
    if confirmed.is_empty() {
        println!("[错误] 代拟名单无有效模块");
        return None;
    }
    let ok = prompt("确认名单？（yes 开始 / 其他取消）");
    if ok.eq_ignore_ascii_case("yes") {
        Some(confirmed)
    } else {
        println!("[取消] 已按用户意愿取消");
        None
    }
}

fn print_review(exec: &orchestrator::Execution) {
    println!("[验收]");
    if exec.items.is_empty() {
        println!("（清单解析失败，原文如下）\n{}", exec.checklist_raw);
        return;
    }
    for i in &exec.items {
        println!("  [{}] {} {}", i.status.to_uppercase(), i.item,
            i.reason.clone().unwrap_or_else(|| i.evidence.clone().unwrap_or_default()));
    }
}

// ---------- 模式三：全能模式 ----------

fn omni_mode(roster: &module::Roster, ids: &str, registry: &providers::Registry) {
    let chosen: Vec<&module::Module> = if ids.is_empty() {
        roster.modules.iter().collect()
    } else {
        ids.split(|c| c == ',' || c == '，')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id))
            .collect()
    };
    if chosen.is_empty() {
        println!("[错误] 无可拼装模块");
        return;
    }
    let mut merged = String::from("你是全能助手，能力由以下模块职责拼装：\n");
    for m in &chosen {
        merged.push_str(&format!("\n== {} ==\n{}", m.manifest.id, m.manifest.system));
    }
    let (mut chat, note) = match registry.resolve(None, None) {
        Some((_, p)) => (Channel::Real(HttpChat { provider: p, model: p.models.first().cloned().unwrap_or_else(|| "default".to_string()) }), None),
        None => (Channel::Demo(FakeChat::new(vec![FakeChat::say("（全能演示）收到。")])), Some("未配置供应商，使用内置假模型演示".to_string())),
    };
    if let Some(n) = note {
        println!("[提示] {}", n);
    }
    println!("[全能] 已拼装 {} 个模块的职责提示词（空行结束会话）。", chosen.len());
    let mut history: Vec<Msg> = vec![Msg::system(merged)];
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        history.push(Msg::user(say));
        let raw = chat.complete(&history);
        let reply = envelope::parse(&raw);
        println!("全能> {}", reply.text);
        history.push(Msg::assistant(reply.text));
    }
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
