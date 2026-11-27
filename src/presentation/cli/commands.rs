//! 目的：终端命令：诊断、密钥、常驻、共享区、回档等一次性命令，跑完即回。
//! 管：命令各自的取参（JSON，来自声明）、调用入站面与就地打印。
//! 不管：词法与校验（声明派生，见 cli/shell.rs）、交互回合（`gate`）、会话流（`flows`）、渲染细节（`view`）。
//! 联动：`cli/shell.rs` 的分派、`view` 的打印原语

use super::*;
use serde_json::{Value, json};

/// 目的：从已校验的参数里取一个字符串（缺省 = None）。
pub(super) fn text(args: &Value, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 目的：从已校验的参数里取一个无符号整数（缺省 = None）。
pub(super) fn num(args: &Value, name: &str) -> Option<u64> {
    args.get(name).and_then(Value::as_u64)
}

/// 目的：只读诊断一条虚拟机档会话的 guest：输出结构化 JSON（进程 / CPU / 内存 / 负载 / 挂载）。
pub(super) fn diagnose_cmd(faces: &Faces, args: &Value, json: bool) -> i32 {
    let Some(sid) = text(args, "session_id") else {
        println!("[用法] diagnose <会话>");
        return 2;
    };
    match faces.actions.act(ActionCall {
        id: "guest_diagnose".to_string(),
        args: json!({ "session_id": sid }),
        caller: Caller::User,
        out: Output::Final,
    }) {
        Ok(Acted::Done(v)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else {
                match serde_json::to_string_pretty(&v) {
                    Ok(text) => println!("{}", text),
                    Err(e) => println!("[错误] 诊断结果序列化失败：{}", e),
                }
            }
            0
        }
        Ok(_) => {
            println!("[完成]");
            0
        }
        Err(e) => {
            println!("[错误] {}", e);
            1
        }
    }
}

/// 目的：解释一条会话的工具面：每个 id 从哪个来源来、授权判定与理由、面里的孤儿 id。
/// 参数：`faces` = 入站面；`args` = 已校验的命令参数（`session` 必填，`agent` / `loop` / `role` 可选）；`json` = 打原始 JSON。
/// 返回：进程退出码（0 成功，1 出错，2 用法不对）。
pub(super) fn explain_cmd(faces: &Faces, args: &Value, json: bool) -> i32 {
    let Some(sid) = text(args, "session") else {
        println!("[用法] explain <会话> [--agent <名>] [--loop <段>] [--role <角色>]");
        return 2;
    };
    let mut payload = json!({ "session_id": sid });
    for key in ["agent", "loop", "role"] {
        if let Some(v) = text(args, key) {
            payload[key] = Value::String(v);
        }
    }
    match faces.actions.act(ActionCall {
        id: "explain_tools".to_string(),
        args: payload,
        caller: Caller::User,
        out: Output::Final,
    }) {
        Ok(Acted::Done(v)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else {
                for line in view::explain_lines(&v) {
                    term::line(&line);
                }
            }
            0
        }
        Ok(_) => {
            println!("[完成]");
            0
        }
        Err(e) => {
            println!("[错误] {}", e);
            1
        }
    }
}

/// 目的：列模块声明的隐秘字段；置值与清除都走统一动作面（值不回显）。
pub(super) fn secret_cmd(faces: &Faces, args: &Value, json: bool) -> i32 {
    match text(args, "action").as_deref() {
        None | Some("list") => secret_list(faces, json),
        Some(action @ ("set" | "clear")) => {
            let (Some(module), Some(name)) = (text(args, "module"), text(args, "name")) else {
                println!("[用法] secret {action} <模块> <名字>");
                return 2;
            };
            let mut payload = json!({ "module": module, "name": name });
            if action == "set" {
                let Some(value) = text(args, "value") else {
                    println!("[提示] 值要在具名旗上给：--value <值>（shell 历史里会留痕）");
                    return 2;
                };
                payload["value"] = Value::String(value);
            }
            let id = if action == "set" {
                "set_secret"
            } else {
                "clear_secret"
            };
            act_and_print(
                faces,
                ActionCall {
                    id: id.to_string(),
                    args: payload,
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some(other) => {
            println!(
                "[用法] secret 的动作只有 list / set / clear（收到 {}）",
                other
            );
            2
        }
    }
}

/// 目的：密钥声明的清单（值不回显，只看得到是否已配置）。
fn secret_list(faces: &Faces, json: bool) -> i32 {
    let list = match faces.secrets.declared() {
        Ok(list) => list,
        Err(e) => {
            println!("[错误] {}", e);
            return 1;
        }
    };
    if json {
        let rows: Vec<Value> = list
            .iter()
            .map(|s| {
                json!({
                    "module": s.module,
                    "name": s.name,
                    "env": s.env,
                    "configured": s.configured,
                    "desc": s.desc,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "secrets": rows })).unwrap_or_default()
        );
        return 0;
    }
    if list.is_empty() {
        println!("（没有声明任何隐秘字段）");
        return 0;
    }
    for s in &list {
        println!(
            "  {:<24} {:<16} {:<8} {}",
            format!("{}.{}", s.module, s.name),
            s.env,
            if s.configured {
                "已配置"
            } else {
                "未配置"
            },
            s.desc
        );
    }
    0
}

/// 目的：常驻服务面板——列服务与状态，启停 / 开关，调用已启动服务的操作（都走统一动作面）。
pub(super) fn resident_cmd(faces: &Faces, args: &Value, json: bool) -> i32 {
    match text(args, "action").as_deref() {
        None | Some("list") => resident_list(faces, json),
        Some(action @ ("start" | "stop" | "enable" | "disable")) => {
            let (Some(module), Some(service)) = (text(args, "module"), text(args, "service"))
            else {
                println!("[用法] resident start|stop|enable|disable <模块id> <服务名>");
                return 2;
            };
            let code = act_and_print(
                faces,
                ActionCall {
                    id: "control_resident".to_string(),
                    args: json!({
                        "module": module,
                        "name": service,
                        "action": action,
                        "lease": "",
                    }),
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            );
            // 启停之后顺手刷新清单：状态是用户最想看的。
            if !json {
                resident_list(faces, false);
            }
            code
        }
        Some("call") => {
            let (Some(module), Some(service), Some(op)) = (
                text(args, "module"),
                text(args, "service"),
                text(args, "op"),
            ) else {
                println!("[用法] resident call <模块id> <服务名> <操作名> [json 参数]");
                return 2;
            };
            let raw = text(args, "payload").unwrap_or_default();
            let params: Value = if raw.trim().is_empty() {
                json!({})
            } else {
                match serde_json::from_str(&raw) {
                    Ok(v) => v,
                    Err(e) => {
                        println!("[错误] 参数不是合法 JSON：{}", e);
                        return 2;
                    }
                }
            };
            act_and_print(
                faces,
                ActionCall {
                    id: format!("module.{}.{}.{}", module, service, op),
                    args: params,
                    caller: Caller::User,
                    out: Output::Final,
                },
                json,
            )
        }
        Some(other) => {
            println!(
                "[提示] 未知子命令 {}（list | start | stop | enable | disable | call）",
                other
            );
            2
        }
    }
}

/// 目的：列出常驻服务（模块 / 服务 / 适配器 / 状态；失败态带如实原因）。
pub(super) fn resident_list(faces: &Faces, json: bool) -> i32 {
    let list = match faces.residents.services() {
        Ok(list) => list,
        Err(e) => {
            println!("[错误] {}", e);
            return 1;
        }
    };
    if json {
        let rows: Vec<Value> = list
            .iter()
            .map(|s| {
                json!({
                    "module": s.module,
                    "name": s.name,
                    "adapter": s.adapter,
                    "state": s.state.label(),
                    "reason": s.reason,
                    "operations": s.operations.iter().map(|o| o.name.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "services": rows })).unwrap_or_default()
        );
        return 0;
    }
    if list.is_empty() {
        println!("（没有声明任何常驻服务）");
        return 0;
    }
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
    0
}

/// 目的：按界面语言渲染一条工具回执（正文原文 + 说明），并按严重度给出成功 / 警告 / 失败标签。
/// 约束：正文由工具进程写出、原样打印；说明按稳定码从目录渲染，不在这里拼句子。
pub(super) fn print_receipt(faces: &Faces, v: &Value) {
    let lang = match faces.registry.settings() {
        Ok(s) => crate::units::business::runtime::api::i18n::pick(&s.lang),
        Err(_) => crate::units::business::runtime::api::i18n::Lang::source(),
    };
    let body = v.get("output").and_then(|s| s.as_str()).unwrap_or("");
    let notes: Vec<crate::units::business::runtime::api::Note> = v
        .get("notes")
        .and_then(|n| serde_json::from_value(n.clone()).ok())
        .unwrap_or_default();
    let severity = v
        .get("severity")
        .and_then(|s| {
            serde_json::from_value::<crate::units::business::runtime::api::Severity>(s.clone()).ok()
        })
        .unwrap_or(if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(true) {
            crate::units::business::runtime::api::Severity::Info
        } else {
            crate::units::business::runtime::api::Severity::Error
        });
    let text = crate::units::business::runtime::api::i18n::render_receipt(lang, body, &notes);
    if !text.trim().is_empty() {
        println!("{}", text);
    }
    println!(
        "[{}]",
        crate::units::business::runtime::api::i18n::label(lang, severity)
    );
}

/// 目的：把一次动作的结果按 CLI 习惯打出来（`--json` 时原样给机器读的结果）；返回退出码。
pub(super) fn act_and_print(faces: &Faces, call: ActionCall, json: bool) -> i32 {
    match faces.actions.act(call) {
        Ok(Acted::Done(v)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return 0;
            }
            if let Some(list) = v.get("operations").and_then(|o| o.as_array()) {
                let names: Vec<&str> = list.iter().filter_map(|x| x.as_str()).collect();
                println!("[完成] 发现操作：{}", names.join("、"));
                return 0;
            }
            if v.get("output").is_some() || v.get("notes").is_some() {
                print_receipt(faces, &v);
                return 0;
            }
            println!("[完成]");
            0
        }
        Ok(Acted::Replayed(events)) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Value::Array(events)).unwrap_or_default()
                );
                return 0;
            }
            println!("[完成]");
            0
        }
        Ok(Acted::Advanced(_)) => {
            println!("[提示] 这条动作需要一条会话才能跑：在 TUI / Web 里用，或让 agent 调");
            2
        }
        Err(e) => {
            println!("[错误] {}", e);
            1
        }
    }
}

/// 目的：共享区提交历史：`work <会话> [log [条数] | show <提交号> | restore <提交号> --yes]`。
///   回滚是非破坏性的：目标之后的提交仍保留，可再次回滚；只改共享区主副本，不动 agent 沙箱。
pub(super) fn work_cmd(faces: &Faces, args: &Value, json: bool) -> i32 {
    let Some(sid) = text(args, "session") else {
        println!("[用法] work <会话> [log [条数] | show <提交号> | restore <提交号> --yes]");
        return 2;
    };
    let action = text(args, "action").unwrap_or_else(|| "log".to_string());
    let rev = num(args, "rev");
    if action == "log" {
        let limit = rev.unwrap_or(20) as usize;
        let log = match faces.work.work_log(&sid, limit, None) {
            Ok(l) => l,
            Err(e) => {
                println!("[错误] {}", e);
                return 1;
            }
        };
        if json {
            println!("{}", serde_json::to_string_pretty(&log).unwrap_or_default());
            return 0;
        }
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
        return 0;
    }
    let Some(id) = rev else {
        println!("[用法] work <会话> show|restore <提交号>");
        return 2;
    };
    match action.as_str() {
        "show" => match faces.work.work_commit_view(&sid, id) {
            Ok(c) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&c).unwrap_or_default());
                    return 0;
                }
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
                            crate::units::business::workspace::api::ChangeKind::Add => "新增",
                            crate::units::business::workspace::api::ChangeKind::Modify => "修改",
                            crate::units::business::workspace::api::ChangeKind::Delete => "删除",
                        },
                        ch.size,
                        ch.path
                    );
                }
                if c.changes_truncated {
                    println!("  （改动太多，列表已截断）");
                }
                0
            }
            Err(e) => {
                println!("[错误] {}", e);
                1
            }
        },
        "restore" => {
            let log = match faces.work.work_log(&sid, 1, None) {
                Ok(l) => l,
                Err(e) => {
                    println!("[错误] {}", e);
                    return 1;
                }
            };
            let head = log.head.as_ref().map(|h| h.id).unwrap_or(0);
            let target = match faces.work.work_commit_view(&sid, id) {
                Ok(c) => c,
                Err(e) => {
                    println!("[错误] {}", e);
                    return 1;
                }
            };
            let yes = args.get("yes").and_then(Value::as_bool).unwrap_or(false);
            if json && !yes {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "head": head,
                        "target": target.id,
                        "restored": false,
                        "hint": "确认回滚请加 --yes",
                    }))
                    .unwrap_or_default()
                );
                return 2;
            }
            if !json {
                println!(
                    "把共享区从 head #{} 回滚到提交 #{}（{}）。目标之后的提交仍保留，可再次回滚；各 agent 沙箱未改动。",
                    head,
                    target.id,
                    first_line(&target.message)
                );
            }
            let confirmed = yes || (!json && prompt("确认回滚？输入 yes：") == "yes");
            if !confirmed {
                println!("[取消] 没有回滚");
                return 0;
            }
            match faces.actions.act(ActionCall {
                id: "restore_work_commit".to_string(),
                args: json!({
                    "session_id": sid,
                    "commit": id,
                    "expect_head": head,
                }),
                caller: Caller::User,
                out: Output::Final,
            }) {
                Ok(Acted::Done(v)) => {
                    let now = v.get("head").and_then(|h| h.as_u64()).unwrap_or(0);
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({ "head": now }))
                                .unwrap_or_default()
                        );
                    } else {
                        println!("[回滚] 共享区现在 head #{}", now);
                    }
                    0
                }
                Ok(_) => {
                    println!("[回滚] 完成");
                    0
                }
                Err(e) => {
                    println!("[错误] {}", e);
                    1
                }
            }
        }
        other => {
            println!("[用法] work <会话> log|show|restore（不认 {}）", other);
            2
        }
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

/// 目的：回档：`rewind <会话> archive|delete|restore <id>`——同一份动作声明，CLI 只把参数装好。
pub(super) fn rewind_cmd(faces: &Faces, args: &Value, json: bool) -> i32 {
    let (Some(sid), Some(mode), Some(id)) =
        (text(args, "session"), text(args, "mode"), num(args, "id"))
    else {
        println!("[用法] rewind <会话> <archive|delete|restore> <id>");
        return 2;
    };
    match faces.actions.act(ActionCall {
        id: "rewind".to_string(),
        args: json!({ "session_id": sid, "mode": mode, "id": id }),
        caller: Caller::User,
        out: Output::Final,
    }) {
        Ok(Acted::Replayed(events)) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Value::Array(events)).unwrap_or_default()
                );
                return 0;
            }
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
            0
        }
        Ok(Acted::Done(v)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else {
                println!("[回档] 完成");
            }
            0
        }
        Ok(Acted::Advanced(_)) => {
            println!("[提示] 回档需要一条会话才能跑");
            2
        }
        Err(e) => {
            println!("[错误] {}", e);
            1
        }
    }
}
