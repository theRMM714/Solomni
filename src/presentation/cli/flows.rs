//! 目的：会话流：单 agent、代理、协作、供应商、模型与核心设置各一条交互流程。
//! 管：每条流程的接线与收尾；共用的小输入助手也在这里。
//! 不管：命令分发、事件渲染、裁决回合。
//! 联动：`cli/gate.rs`、`cli/view.rs`、`cli/commands.rs`

use super::*;
use crate::presentation::faces::Faces;
use serde_json::{Value, json};

/// 目的：单 agent 会话：点名的 agent 一起干同一件事，进转录提示符直到空行。
pub(super) fn single_flow(faces: &Faces, args: &Value) {
    let views = match faces.registry.agents() {
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
    let names = text(args, "agents").unwrap_or_default();
    let picked: Vec<AgentMeta> = if names.trim().is_empty() {
        views
            .iter()
            .map(crate::capabilities::catalog::registry::api::agent_meta)
            .collect()
    } else {
        match pick_agents(faces, &split_names(&names)) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let work_name = match faces.work.unique_work_name("single", "single") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    // 档位：CLI 不提供交互选择，用设置里的默认档（与 Web 向导的默认一致）。
    let tier = faces
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    // 订阅起点：命令回包只给头部序号，事实一律从事件台按 since 取。
    let mut cursor = faces.events.head();
    let sid = match create_session_action(faces, &work_name, "single", &picked, None, tier) {
        Ok(sid) => sid,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    cursor = drain(faces, &sid, cursor);
    println!("（单 agent {} —— 输入消息，空行结束会话）", sid);
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        // 终端只在最终结果上渲染，不要流式（怎么显示是呈现层的事）。
        match act_interactive(
            faces,
            &sid,
            CliAction::Say(say.clone()),
            &mut cursor,
            Output::Final,
        ) {
            Ok(acted) => follow(faces, &sid, &mut cursor, acted),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

/// 目的：把决定权整块交给核心：**没有名单**（核心自己挑人、建子工作），接着就是跟它对话。 选这一形态本身就是**授予全权**（`WorkMode::Proxy` → `meta.delegation`）。
pub(super) fn proxy_flow(faces: &Faces) {
    let work_name = match faces.work.unique_work_name("proxy", "proxy") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let tier = faces
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    // 订阅起点：命令回包只给头部序号，事实一律从事件台按 since 取。
    let mut cursor = faces.events.head();
    let sid = match create_session_action(faces, &work_name, "proxy", &[], None, tier) {
        Ok(sid) => sid,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    cursor = drain(faces, &sid, cursor);
    println!("（核心代理 {} —— 输入消息，空行结束会话）", sid);
    loop {
        let say = prompt("你>");
        if say.is_empty() {
            break;
        }
        match act_interactive(
            faces,
            &sid,
            CliAction::Say(say.clone()),
            &mut cursor,
            Output::Final,
        ) {
            Ok(acted) => follow(faces, &sid, &mut cursor, acted),
            Err(e) => {
                println!("[错误] {}", e);
                break;
            }
        }
    }
}

/// 目的：协作会话：给需求、点名单（或让核心代拟），之后一律按裁决卡推进。
pub(super) fn collab_flow(faces: &Faces, args: &Value) {
    let names = text(args, "agents").unwrap_or_default();
    let trimmed = names.trim();
    let delegate = trimmed.is_empty() || trimmed == "?";
    let task = prompt("需求>");
    // 代拟（无参或 ?）= 核心拟名单；点名 = 用登记处里的那几个 agent。
    let agents: Vec<AgentMeta> = if delegate {
        Vec::new()
    } else {
        match pick_agents(faces, &split_names(trimmed)) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    };
    let work_name = match faces.work.unique_work_name("collab", "collab") {
        Ok(n) => n,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let tier = faces
        .registry
        .settings()
        .map(|s| s.tier)
        .unwrap_or(Tier::Host);
    let mut cursor = faces.events.head();
    // 代拟由形态派生（协作 + 没给名单 = 核心按需求拟名单）；CLI 只交点名结果。
    let sid = match create_session_action(faces, &work_name, "collab", &agents, Some(&task), tier) {
        Ok(sid) => {
            cursor = drain(faces, &sid, cursor);
            sid
        }
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };

    // 裁决门：**按卡片上的选项作答**（名单确认 / 开始讨论 / 请教 / 方案待审 / 节点没过都走这里）。
    answer_gates(faces, &sid, &mut cursor);
}

/// 目的：供应商登记：列清单、登记（含只换密钥）、移除、按端点发现可用模型——写操作一律落动作面。
pub(super) fn provider_flow(faces: &Faces, args: &Value, json: bool) -> i32 {
    match text(args, "action").as_deref() {
        None | Some("list") => {
            let views = match faces.registry.providers() {
                Ok(v) => v,
                Err(e) => {
                    println!("[错误] {}", e);
                    return 1;
                }
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "providers": views }))
                        .unwrap_or_default()
                );
                return 0;
            }
            if views.is_empty() {
                println!("（无供应商）用 provider add <id> <base_url> --api-key <key> 添加");
            }
            for p in &views {
                println!("  {}", provider_line(p));
            }
            0
        }
        Some(action @ ("add" | "key")) => {
            let Some(id) = text(args, "id") else {
                println!(
                    "[用法] provider {} <id> [--base-url <地址>] [--api-key <密钥>]",
                    action
                );
                return 2;
            };
            // key = 只换密钥：端点沿用登记处里已有的那一条。
            let base_url = match text(args, "base_url") {
                Some(u) => u,
                None => match faces
                    .registry
                    .providers()
                    .ok()
                    .and_then(|v| v.into_iter().find(|p| p.id == id))
                {
                    Some(p) => p.base_url,
                    None => {
                        println!(
                            "[错误] 无此供应商：{}（先 provider add {} <地址> --api-key <密钥>）",
                            id, id
                        );
                        return 1;
                    }
                },
            };
            let mut payload = json!({ "id": id, "base_url": base_url });
            if let Some(key) = text(args, "api_key") {
                payload["api_key"] = Value::String(key);
            }
            act_and_print(
                faces,
                ActionCall {
                    id: "upsert_provider".to_string(),
                    args: payload,
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some("rm") => {
            let Some(id) = text(args, "id") else {
                println!("[用法] provider rm <id>");
                return 2;
            };
            act_and_print(
                faces,
                ActionCall {
                    id: "remove_provider".to_string(),
                    args: json!({ "id": id }),
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some("discover") => {
            let Some(id) = text(args, "id") else {
                println!("[用法] provider discover <id>");
                return 2;
            };
            act_and_print(
                faces,
                ActionCall {
                    id: "discover_models".to_string(),
                    args: json!({ "id": id }),
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some(other) => {
            println!(
                "[提示] 未知子命令 {}（list | add | key | rm | discover）",
                other
            );
            2
        }
    }
}

/// 目的：模型登记：列清单（标出核心默认）、登记、移除——写操作一律落动作面。
pub(super) fn model_flow(faces: &Faces, args: &Value, json: bool) -> i32 {
    match text(args, "action").as_deref() {
        None | Some("list") => {
            let views = match faces.registry.models() {
                Ok(v) => v,
                Err(e) => {
                    println!("[错误] {}", e);
                    return 1;
                }
            };
            let core = faces.registry.core_model().ok().flatten();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "models": views,
                        "core": core,
                    }))
                    .unwrap_or_default()
                );
                return 0;
            }
            if views.is_empty() {
                println!(
                    "（无模型）用 model add <id> <展示名> <实际模型串> <供应商id> [备注] 添加"
                );
            }
            for m in &views {
                println!("  {}", model_line(m, core.as_deref()));
            }
            0
        }
        Some("add") => {
            let (Some(id), Some(name), Some(api_model), Some(provider)) = (
                text(args, "id"),
                text(args, "name"),
                text(args, "api_model"),
                text(args, "provider"),
            ) else {
                println!("[用法] model add <id> <展示名> <实际模型串> <供应商id> [备注]");
                return 2;
            };
            let mut payload = json!({
                "id": id,
                "name": name,
                "api_model": api_model,
                "provider": provider,
            });
            if let Some(note) = text(args, "note") {
                payload["note"] = Value::String(note);
            }
            act_and_print(
                faces,
                ActionCall {
                    id: "upsert_model".to_string(),
                    args: payload,
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some("rm") => {
            let Some(id) = text(args, "id") else {
                println!("[用法] model rm <id>");
                return 2;
            };
            act_and_print(
                faces,
                ActionCall {
                    id: "remove_model".to_string(),
                    args: json!({ "id": id }),
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some(other) => {
            println!("[提示] 未知子命令 {}（list | add | rm）", other);
            2
        }
    }
}

/// 目的：核心默认模型：不带参数看当前，带参数切过去（切换落动作面）。
pub(super) fn core_flow(faces: &Faces, args: &Value, json: bool) -> i32 {
    let Some(model) = text(args, "model") else {
        let cur = faces.registry.core_model().ok().flatten();
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "core": cur })).unwrap_or_default()
            );
        } else {
            println!(
                "核心默认模型：{}",
                cur.unwrap_or_else(|| "（未设定）".to_string())
            );
        }
        return 0;
    };
    act_and_print(
        faces,
        ActionCall {
            id: "set_core_model".to_string(),
            args: json!({ "id": model }),
            caller: Caller::User,
            out: Output::Final,
        },
        json,
    )
}

/// 目的：CLI 读输入读到尽头（EOF）时，把挂着的裁决按**停止 = 拒绝**收场——前端负责解开等待方。
/// 返回：真的停下了这条（整棵）会话。
/// 约束：停止 = 拒绝（整队作废）；不套用发起方声明的默认项——那会把"没人答"变成放行。
pub(crate) fn eof_stop(faces: &Faces, sid: &str) -> bool {
    println!("[提示] 输入已到尽头：挂着的裁决按「停止 = 拒绝」收场，这一趟不继续。");
    !faces.runtime.stop(sid).is_empty()
}

/// 目的：读一行输入；**读到尽头（EOF）给 `None`**——"用户不在键盘前"与"用户先不答"是两件事。
pub(super) fn prompt_opt(text: &str) -> Option<String> {
    term::ask(text)
}

pub(super) fn prompt(text: &str) -> String {
    prompt_opt(text).unwrap_or_default()
}

pub(super) fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
