//! 目的：终端命令：模块、诊断、密钥、常驻、共享区、回档等一次性命令，跑完即回。
//! 管：命令各自的参数解析、调用能力面与就地打印。
//! 不管：交互回合（`gate`）、会话流（`flows`）、渲染细节（`view`）。
//! 联动：`cli/mod.rs` 的命令分发、`view` 的打印原语

use super::*;

pub(super) fn module_cmd(ops: &Ops, arg: &str) {
    let catalog = match ops.actions.catalog(&Caller::User, None) {
        Ok(c) => c,
        Err(e) => {
            println!("[错误] {}", e);
            return;
        }
    };
    let modules: Vec<_> = catalog
        .iter()
        .filter(|a| a.id.starts_with("module."))
        .collect();
    let arg = arg.trim();
    if arg.is_empty() || arg == "list" {
        // 按**模块**分组：声明的 tools / services 就是它的独立入口；只写 system 的模块如实标注。
        match ops.modules.roster() {
            Ok(roster) => {
                for m in &roster.modules {
                    let prefix = format!("module.{}.", m.manifest.id);
                    let acts: Vec<_> = modules
                        .iter()
                        .filter(|a| a.id.starts_with(&prefix))
                        .collect();
                    let declares = !m.manifest.tools.is_empty() || !m.manifest.services.is_empty();
                    if !declares {
                        println!(
                            "  {:<20} （只能经 AI 使用：没有声明 tools / services）",
                            m.manifest.id
                        );
                        continue;
                    }
                    if acts.is_empty() {
                        println!(
                            "  {:<20} （声明的服务还没启动：先 resident start）",
                            m.manifest.id
                        );
                        continue;
                    }
                    println!("  {}：", m.manifest.id);
                    for a in acts {
                        println!(
                            "      {:<28} {}",
                            a.id.trim_start_matches(&prefix),
                            first_line(&a.desc)
                        );
                    }
                }
            }
            Err(e) => println!("[错误] {}", e),
        }
        println!(
            "用法：module <模块id>.<工具名> [json 参数]（省略 = {{}}；可选 workspace = 工作目录）"
        );
        return;
    }
    let (id, raw) = match arg.split_once(char::is_whitespace) {
        Some((id, rest)) => (id, rest.trim()),
        None => (arg, ""),
    };
    let args: serde_json::Value = if raw.is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                println!("[错误] 参数不是合法 JSON：{}", e);
                return;
            }
        }
    };
    match ops.actions.act(ActionCall {
        id: module_action_id(id),
        args,
        caller: Caller::User,
        out: Output::Final,
    }) {
        Ok(Acted::Done(v)) => {
            let ok = v.get("ok").and_then(|b| b.as_bool()).unwrap_or(true);
            println!("{}", v.get("output").and_then(|s| s.as_str()).unwrap_or(""));
            if !ok {
                println!("[失败] 模块工具返回 ok=false（回执见上）");
            }
        }
        Ok(_) => println!("[完成]"),
        Err(e) => println!("[错误] {}", e),
    }
}

/// 目的：只读诊断一条虚拟机档会话的 guest：输出结构化 JSON（进程 / CPU / 内存 / 负载 / 挂载）。
pub(super) fn diagnose_cmd(ops: &Ops, arg: &str) {
    let sid = arg.trim();
    if sid.is_empty() {
        println!("用法：diagnose <会话>");
        return;
    }
    match ops.actions.act(ActionCall {
        id: "guest_diagnose".to_string(),
        args: serde_json::json!({ "session_id": sid }),
        caller: Caller::User,
        out: Output::Final,
    }) {
        Ok(Acted::Done(v)) => match serde_json::to_string_pretty(&v) {
            Ok(text) => println!("{}", text),
            Err(e) => println!("[错误] 诊断结果序列化失败：{}", e),
        },
        Ok(_) => println!("[完成]"),
        Err(e) => println!("[错误] {}", e),
    }
}

/// 目的：命令行回档：给共享区与整棵子树都对齐到同一个点。 留档 = 标记 + 折叠（可恢复）；删除 = 真的截掉；恢复 = 删掉该标记及其后（不可恢复）。
/// 目的：列出模块声明的隐秘字段（是否已配置）；置值随设置面接入。
pub(super) fn secret_cmd(ops: &Ops) {
    match ops.secrets.declared() {
        Ok(list) if list.is_empty() => println!("（没有声明任何隐秘字段）"),
        Ok(list) => {
            for s in &list {
                let mark = if s.configured {
                    "已配置"
                } else {
                    "未配置"
                };
                println!(
                    "  {:<24} {:<16} {:<8} {}",
                    format!("{}.{}", s.module, s.name),
                    s.env,
                    mark,
                    s.desc
                );
            }
        }
        Err(e) => println!("[错误] {}", e),
    }
}

/// 目的：常驻服务面板——列服务与状态，启停 / 开关，调用已启动服务的操作（都走统一动作面）。
pub(super) fn resident_cmd(ops: &Ops, arg: &str) {
    let parts: Vec<&str> = arg.split_whitespace().collect();
    match parts.first().map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("list") => resident_list(ops),
        Some(v @ ("start" | "stop" | "enable" | "disable")) => {
            let (Some(module), Some(name)) = (parts.get(1), parts.get(2)) else {
                println!("[用法] resident start|stop|enable|disable <模块id> <服务名> [租约]");
                return;
            };
            let lease = parts.get(3).copied().unwrap_or("");
            act_and_print(
                ops,
                ActionCall {
                    id: "control_resident".to_string(),
                    args: serde_json::json!({ "module": module, "name": name, "action": v, "lease": lease }),
                    caller: Caller::User,
                    out: Output::Final,
                },
            );
            // 启停之后顺手刷新清单：状态是用户最想看的。
            resident_list(ops);
        }
        Some("call") => {
            let (Some(module), Some(service), Some(op)) =
                (parts.get(1), parts.get(2), parts.get(3))
            else {
                println!("[用法] resident call <模块id> <服务名> <操作名> [json 参数]");
                return;
            };
            let raw = parts.get(4..).map(|s| s.join(" ")).unwrap_or_default();
            let args: serde_json::Value = if raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(&raw) {
                    Ok(v) => v,
                    Err(e) => {
                        println!("[错误] 参数不是合法 JSON：{}", e);
                        return;
                    }
                }
            };
            act_and_print(
                ops,
                ActionCall {
                    id: format!("module.{}.{}.{}", module, service, op),
                    args,
                    caller: Caller::User,
                    out: Output::Final,
                },
            );
        }
        Some(other) => println!(
            "[提示] 未知子命令 {}（list | start | stop | enable | disable | call）",
            other
        ),
    }
}

/// 目的：列出常驻服务（模块 / 服务 / 适配器 / 状态；失败态带如实原因）。
pub(super) fn resident_list(ops: &Ops) {
    match ops.residents.services() {
        Ok(list) if list.is_empty() => println!("（没有声明任何常驻服务）"),
        Ok(list) => {
            for s in &list {
                let note = if s.reason.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", s.reason)
                };
                println!(
                    "  {:<24} {:<12} {}{}",
                    format!("{}.{}", s.module, s.name),
                    s.adapter,
                    s.state.label(),
                    note
                );
            }
        }
        Err(e) => println!("[错误] {}", e),
    }
}

/// 目的：把一次动作的结果按 CLI 习惯打出来（发现的操作 / 回执 / 完成 / 错误）。
pub(super) fn act_and_print(ops: &Ops, call: ActionCall) {
    match ops.actions.act(call) {
        Ok(Acted::Done(v)) => {
            if let Some(list) = v.get("operations").and_then(|o| o.as_array()) {
                let names: Vec<&str> = list.iter().filter_map(|x| x.as_str()).collect();
                println!("[完成] 发现操作：{}", names.join("、"));
                return;
            }
            if let Some(out) = v.get("output").and_then(|s| s.as_str()) {
                println!("{}", out);
                if !v.get("ok").and_then(|b| b.as_bool()).unwrap_or(true) {
                    println!("[失败] 服务返回 ok=false（回执见上）");
                }
                return;
            }
            println!("[完成]");
        }
        Ok(_) => println!("[完成]"),
        Err(e) => println!("[错误] {}", e),
    }
}

/// 目的：共享区提交历史：`work <会话> [log [条数] | show <提交号> | restore <提交号> [--yes]]`。 回滚是非破坏性的：目标之后的提交仍保留，可再次回滚；只改共享区主副本，不动 agent 沙箱。
pub(super) fn work_cmd(ops: &Ops, arg: &str) {
    let parts: Vec<&str> = arg.split_whitespace().collect();
    let Some(sid) = parts.first().copied() else {
        println!("[用法] work <会话> [log [条数] | show <提交号> | restore <提交号> [--yes]]");
        return;
    };
    let verb = parts
        .get(1)
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "log".to_string());
    if verb == "log" {
        let limit = parts
            .get(2)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(20);
        let log = match ops.sessions.work_log(sid, limit, None) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        };
        match &log.head {
            Some(h) => println!(
                "共享区 head #{}（{}，{}）· {} 个提交",
                h.id,
                h.author,
                time_label(h.time),
                log.total
            ),
            None => println!("共享区还没有提交"),
        }
        for c in &log.commits {
            println!(
                "  {} #{:<4} {:<10} {:<10} {} 处改动  {}",
                if c.on_head { "*" } else { " " },
                c.id,
                c.author,
                time_label(c.time),
                c.changes,
                first_line(&c.message)
            );
        }
        if log.truncated {
            println!("  （还有更早的提交：work {} log <更多条数>）", sid);
        }
        return;
    }
    let id = match parts.get(2).and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => {
            println!("[用法] work <会话> show|restore <提交号>");
            return;
        }
    };
    match verb.as_str() {
        "show" => match ops.sessions.work_commit_view(sid, id) {
            Ok(c) => {
                println!(
                    "提交 #{}（{}，{}）：{}",
                    c.id,
                    c.author,
                    time_label(c.time),
                    c.message
                );
                for ch in &c.changes {
                    println!(
                        "  {:<7} {:<10} {}",
                        match ch.kind {
                            crate::capabilities::workspace::api::ChangeKind::Add => "新增",
                            crate::capabilities::workspace::api::ChangeKind::Modify => "修改",
                            crate::capabilities::workspace::api::ChangeKind::Delete => "删除",
                        },
                        ch.size,
                        ch.path
                    );
                }
                if c.changes_truncated {
                    println!("  （改动太多，列表已截断）");
                }
            }
            Err(e) => println!("[错误] {}", e),
        },
        "restore" => {
            let log = match ops.sessions.work_log(sid, 1, None) {
                Ok(l) => l,
                Err(e) => {
                    println!("[错误] {}", e);
                    return;
                }
            };
            let head = log.head.as_ref().map(|h| h.id).unwrap_or(0);
            let target = match ops.sessions.work_commit_view(sid, id) {
                Ok(c) => c,
                Err(e) => {
                    println!("[错误] {}", e);
                    return;
                }
            };
            println!(
                "把共享区从 head #{} 回滚到提交 #{}（{}）。目标之后的提交仍保留，可再次回滚；各 agent 沙箱未改动。",
                head, target.id, first_line(&target.message)
            );
            let yes = parts.iter().any(|p| p.eq_ignore_ascii_case("--yes"))
                || prompt("确认回滚？输入 yes：") == "yes";
            if !yes {
                println!("[取消] 没有回滚");
                return;
            }
            match ops.actions.act(ActionCall {
                id: "restore_work_commit".to_string(),
                args: serde_json::json!({
                    "session_id": sid,
                    "commit": id,
                    "expect_head": head,
                }),
                caller: Caller::User,
                out: Output::Final,
            }) {
                Ok(Acted::Done(v)) => {
                    let now = v.get("head").and_then(|h| h.as_u64()).unwrap_or(0);
                    println!("[回滚] 共享区现在 head #{}", now);
                }
                Ok(_) => println!("[回滚] 完成"),
                Err(e) => println!("[错误] {}", e),
            }
        }
        other => println!("[用法] work <会话> log|show|restore（不认 {}）", other),
    }
}

/// 目的：提交时间的人读标签：最近用相对时间，较老直接给时间戳（终端不引第三方日期库）。
pub(super) fn time_label(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(ts);
    match diff {
        d if d < 60 => "刚刚".to_string(),
        d if d < 3600 => format!("{} 分钟前", d / 60),
        d if d < 86_400 => format!("{} 小时前", d / 3600),
        d if d < 30 * 86_400 => format!("{} 天前", d / 86_400),
        _ => format!("{}", ts),
    }
}

pub(super) fn rewind_cmd(ops: &Ops, arg: &str) {
    let parts: Vec<&str> = arg.split_whitespace().collect();
    let usage = "[用法] rewind <会话> archive|delete <行id>  或  rewind <会话> restore <标记id>";
    if parts.len() < 3 {
        println!("{}", usage);
        return;
    }
    let (sid, verb) = (parts[0], parts[1].to_ascii_lowercase());
    let num = match parts[2].parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            println!("{}", usage);
            return;
        }
    };
    let mode = match verb.as_str() {
        "archive" | "delete" | "restore" => verb.clone(),
        _ => {
            println!("{}", usage);
            return;
        }
    };
    // 回档也走动作表：同一份声明与同一处授权（CLI 只把参数装好）。
    match ops.actions.act(ActionCall {
        id: "rewind".to_string(),
        args: serde_json::json!({ "session_id": sid, "mode": mode, "id": num }),
        caller: Caller::User,
        out: Output::Final,
    }) {
        Ok(Acted::Replayed(events)) => {
            let rows: Vec<String> = events
                .iter()
                .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("transcript"))
                .filter_map(|e| e.get("lines").and_then(|l| l.as_array()))
                .flatten()
                .map(|l| {
                    format!(
                        "#{} {}",
                        l.get("id").and_then(|i| i.as_u64()).unwrap_or(0),
                        l.get("line").and_then(|x| x.as_str()).unwrap_or("")
                    )
                })
                .collect();
            println!("[回档] 现在 {} 行（尾部）：", rows.len());
            for r in rows.iter().rev().take(10).rev() {
                println!("  {}", r);
            }
        }
        Ok(_) => println!("[回档] 完成"),
        Err(e) => println!("[错误] {}", e),
    }
}
