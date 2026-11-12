//! 目的：会话流：单 agent、代理、协作、供应商、模型与核心设置各一条交互流程。
//! 管：每条流程的接线与收尾；共用的小输入助手也在这里。
//! 不管：命令分发、事件渲染、裁决回合。
//! 联动：`cli/gate.rs`、`cli/view.rs`、`cli/commands.rs`

use super::*;

pub(super) fn single_flow(ops: &Ops, arg: &str) {
    let views = match ops.registry.agents() {
        Ok(v) => v,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    if views.is_empty() {
        println!("[错误] {}（CLI 不再直接点模块）", NO_AGENTS);
        return;
    }
    // 点名 1 个 = 直接用该 agent（模块数不限）；点名多个 / 无参 = 全部。
    // **「多个并成一个临时组合」的组合语义由核心在建工作时收口**（前端只交点名结果）。
    let picked: Vec<AgentInstance> = if arg.trim().is_empty() {
        views.iter().map(AgentInstance::from_view).collect()
    } else {
        match pick_agents(ops, &split_names(arg)) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let work_name = match ops.sessions.unique_work_name("single", "single") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    // 档位：CLI 不提供交互选择，用设置里的默认档（与 Web 向导的默认一致）。
    let tier = ops
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    // 订阅起点：命令回包只给头部序号，事实一律从事件台按 since 取。
    let mut cursor = ops.events.head();
    let sid = match create_session_action(ops, &work_name, "single", &picked, None, tier) {
        Ok(sid) => sid,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    cursor = drain(ops, &sid, cursor);
    println!("（单 agent {} —— 输入消息，空行结束会话）", sid);
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        // 终端只在最终结果上渲染，不要流式（怎么显示是呈现层的事）。
        match act_interactive(
            ops,
            &sid,
            CliAction::Say(say.clone()),
            &mut cursor,
            Output::Final,
        ) {
            Ok(acted) => follow(ops, &sid, &mut cursor, acted),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

/// 目的：把决定权整块交给核心：**没有名单**（核心自己挑人、建子工作），接着就是跟它对话。 选这一形态本身就是**授予全权**（`WorkMode::Proxy` → `meta.delegation`）。
pub(super) fn proxy_flow(ops: &Ops) {
    let work_name = match ops.sessions.unique_work_name("proxy", "proxy") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let tier = ops
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    // 订阅起点：命令回包只给头部序号，事实一律从事件台按 since 取。
    let mut cursor = ops.events.head();
    let sid = match create_session_action(ops, &work_name, "proxy", &[], None, tier) {
        Ok(sid) => sid,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    cursor = drain(ops, &sid, cursor);
    println!("（核心代理 {} —— 输入消息，空行结束会话）", sid);
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        match act_interactive(
            ops,
            &sid,
            CliAction::Say(say.clone()),
            &mut cursor,
            Output::Final,
        ) {
            Ok(acted) => follow(ops, &sid, &mut cursor, acted),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

pub(super) fn collab_flow(ops: &Ops, arg: &str) {
    let trimmed = arg.trim();
    let delegate = trimmed.is_empty() || trimmed == "?";
    let task = prompt("需求>");
    // 代拟（无参或 ?）= 核心拟名单；点名 = 用登记处里的那几个 agent。
    let agents: Vec<AgentInstance> = if delegate {
        Vec::new()
    } else {
        match pick_agents(ops, &split_names(trimmed)) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let work_name = match ops.sessions.unique_work_name("collab", "collab") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let tier = ops
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    let mut cursor = ops.events.head();
    // 代拟由形态派生（协作 + 没给名单 = 核心按需求拟名单）；CLI 只交点名结果。
    let sid = match create_session_action(ops, &work_name, "collab", &agents, Some(&task), tier) {
        Ok(sid) => {
            cursor = drain(ops, &sid, cursor);
            sid
        }
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };

    // 裁决门：**按卡片上的选项作答**（名单确认 / 开始讨论 / 请教 / 方案待审 / 节点没过都走这里）。
    answer_gates(ops, &sid, &mut cursor);
}

pub(super) fn provider_flow(ops: &Ops, arg: &str) {
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

pub(super) fn model_flow(ops: &Ops, arg: &str) {
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
            let draft = crate::capabilities::registry::api::ModelDraft {
                id: w[0].to_string(),
                name: w[1].to_string(),
                api_model: w[2].to_string(),
                provider: w[3].to_string(),
                note: note.to_string(),
                tools: Default::default(),
                context: 0,
                thinking: Default::default(),
            };
            match ops.registry.upsert_model(draft) {
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

pub(super) fn core_flow(ops: &Ops, arg: &str) {
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

/// 目的：CLI 读输入读到尽头（EOF）时，把挂着的裁决按**停止 = 拒绝**收场——前端负责解开等待方。
/// 返回：真的停下了这条（整棵）会话。
/// 约束：停止 = 拒绝（整队作废）；不套用发起方声明的默认项——那会把"没人答"变成放行。
pub(crate) fn eof_stop(ops: &Ops, sid: &str) -> bool {
    println!("[提示] 输入已到尽头：挂着的裁决按「停止 = 拒绝」收场，这一趟不继续。");
    !ops.sessions.stop(sid).is_empty()
}

/// 目的：读一行输入；**读到尽头（EOF）给 `None`**——"用户不在键盘前"与"用户先不答"是两件事。
pub(super) fn prompt_opt(text: &str) -> Option<String> {
    print!("{} ", text);
    std::io::stdout().flush().ok();
    let mut s = String::new();
    match std::io::stdin().read_line(&mut s) {
        Ok(0) => None,
        Ok(_) => Some(s.trim().to_string()),
        Err(_) => None,
    }
}

pub(super) fn prompt(text: &str) -> String {
    prompt_opt(text).unwrap_or_default()
}

pub(super) fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
