//! 目的：交互回合：跟随事件流、投降裁决卡、等安静与动作回显。
//! 管：跟读游标、裁决卡的问与答、动作后的续读。
//! 不管：生成本身（在 `conductor`）、渲染原语（`view`）。
//! 联动：`cli/view.rs`、`cli/flows.rs`、`capabilities/conductor`

use super::*;

/// 目的：**订阅事件台**：把 `from` 之后属于这条会话的批次渲染出来，返回新的游标。 命令回包只给头部序号——事实一条不落都在事件台上，CLI 与 Web 前端读的是同一份。

pub(super) fn drain(ops: &Ops, sid: &str, from: u64) -> u64 {
    let (lines, head, _oldest) = ops.events.snapshot(Some(sid), from);
    for l in &lines {
        render(&l.events);
    }
    head
}

/// 目的：一次命令之后的订阅。回档/改需求给的是**重放快照**（不是增量事实）：终端不重复打， 只把游标推到当前头部（不重渲已有的行）。
pub(super) fn follow(ops: &Ops, sid: &str, cursor: &mut u64, acted: Acted) {
    if matches!(acted, Acted::Replayed(_)) {
        *cursor = ops.events.head();
        return;
    }
    *cursor = drain(ops, sid, *cursor);
}

/// 目的：CLI 侧的动作（拥有字符串）：生成要放后台线程，所以不能借用调用栈上的 `&str`。
pub(super) enum CliAction {
    Say(String),
    /// 回答一张裁决卡：卡号 + 选项 id + 附言（与 Web 回答的是同一条命令）。
    Answer {
        card: String,
        option: String,
        note: String,
    },
}

impl CliAction {
    /// 目的：把 CLI 的动作变成一次**动作调用**（与 Web 同一条分发、同一份声明）。
    fn call(&self, sid: &str, out: Output) -> ActionCall {
        match self {
            CliAction::Say(t) => ActionCall {
                id: "send_message".to_string(),
                args: serde_json::json!({ "session_id": sid, "text": t }),
                caller: Caller::User,
                out,
            },
            CliAction::Answer { card, option, note } => ActionCall {
                id: "answer_card".to_string(),
                args: serde_json::json!({
                    "session_id": sid,
                    "card": card,
                    "option": option,
                    "note": note,
                }),
                caller: Caller::User,
                out,
            },
        }
    }
}

/// 目的：队首之后还在等的那几张，一行一张（谁在等、问的什么、前面还排着几条）。
/// 约束：**文本构造与打印分开**——文本可判，打印只是把它写出去。
pub(crate) fn waiting_lines(waiting: &[DecisionWaiter]) -> Vec<String> {
    if waiting.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "  后面还排着 {} 张（先答上面那张）：",
        waiting.len()
    )];
    for (i, w) in waiting.iter().enumerate() {
        out.push(format!(
            "    {}) {} 在等：{}",
            i + 1,
            w.envelope.name,
            w.title
        ));
    }
    out
}

/// 目的：把还在等的那几张如实打出来（**只有队首能答**）。
pub(super) fn print_waiting(waiting: &[DecisionWaiter]) {
    for line in waiting_lines(waiting) {
        println!("{}", line);
    }
}

/// 目的：把一张裁决卡按它的四个字段打出来（信封 / 消息 / 选项）：CLI 不认识业务含义，只认这几格。
pub(super) fn print_card(card: &DecisionCard) {
    println!("[裁决] {}（{}）", card.message.title, card.envelope.name);
    if !card.message.body.trim().is_empty() {
        println!("  {}", card.message.body);
    }
    if !card.message.detail.trim().is_empty() {
        println!("  {}", card.message.detail);
    }
    for (i, o) in card.options.iter().enumerate() {
        println!("  {}) {}（{}）", i + 1, o.label, o.id);
    }
}

/// 目的：选中的选项 id：先按序号、再按 id 原文认——两者都是那张卡上**用户看得见**的东西。
pub(crate) fn pick_option(card: &DecisionCard, ans: &str) -> Option<String> {
    if let Ok(n) = ans.parse::<usize>() {
        if n >= 1 && n <= card.options.len() {
            return Some(card.options[n - 1].id.clone());
        }
    }
    card.options
        .iter()
        .find(|o| o.id == ans)
        .map(|o| o.id.clone())
}

/// 目的：裁决门：**按卡片上的选项作答**（回答回的是选项 id + 附言，不是自由文本）。 不点不继续：空输入 = 先不答（会话仍在等）。
pub(super) fn answer_gates(ops: &Ops, sid: &str, cursor: &mut u64) {
    loop {
        let queue = match ops.sessions.open_queue(sid) {
            Ok(Some(q)) => q,
            Ok(None) => return,
            Err(e) => {
                println!("[提示] 取裁决卡失败：{}", e);
                return;
            }
        };
        let card = &queue.card;
        print_card(card);
        // 队首之后还在等的那几张：如实列出来（"谁在等、前面还排着几条"），但它们还不能答。
        print_waiting(&queue.waiting);
        let Some(ans) = prompt_opt("选哪一项（序号 / 选项 id；回车 = 先不答）>")
        else {
            // 输入到尽头 = 前端答不了了：按**停止**（= 拒绝）收场，绝不把等待方永远吊在这里。
            eof_stop(ops, sid);
            return;
        };
        let ans = ans.trim();
        if ans.is_empty() {
            return;
        }
        let Some(option) = pick_option(card, ans) else {
            println!("  这张卡上没有这一项，请按上面的序号或 id 作答。");
            continue;
        };
        let note = prompt("附言（可空；请教那一关必填）>");
        let action = CliAction::Answer {
            card: card.id.clone(),
            option,
            note,
        };
        match act_interactive(ops, sid, action, cursor, Output::Final) {
            Ok(acted) => {
                follow(ops, sid, cursor, acted);
                wait_quiet(ops, sid, cursor);
            }
            Err(e) => {
                println!("[错误] {}", e);
                return;
            }
        }
    }
}

/// 目的：回答之后**跟到这一段收尾**再回提示符：回答是直路（先落定、处置脱离调用方跑）， 不跟的话终端会在讨论还在跑的时候就回到提示符——用户既看不到后续，也等不到下一张卡。 判据：这条会话不在跑，且事件台连着几拍没有再长（等的是"脱离调用方那一段"，不是某一次生成）。
pub(super) fn wait_quiet(ops: &Ops, sid: &str, cursor: &mut u64) {
    let mut last = u64::MAX;
    let mut quiet = 0u32;
    loop {
        *cursor = drain(ops, sid, *cursor);
        let head = ops.events.head();
        if !ops.sessions.is_running(sid) && head == last {
            quiet += 1;
            if quiet >= 3 {
                return;
            }
        } else {
            quiet = 0;
        }
        last = head;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// 目的：一次生成放**后台线程**；主线程一边渲染事件、一边就地按卡作答（工具级确认等在工作线程上）。 为什么：回答要在生成进行中读键盘，同步调用会把主线程堵在生成里，读不到输入。
pub(super) fn act_interactive(
    ops: &Ops,
    sid: &str,
    action: CliAction,
    cursor: &mut u64,
    out: Output,
) -> Result<Acted, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (gen_ops, gen_sid) = (ops.clone(), sid.to_string());
    std::thread::spawn(move || {
        let r = gen_ops.actions.act(action.call(&gen_sid, out));
        let _ = tx.send(r);
    });
    loop {
        let (lines, head, _oldest) = ops.events.snapshot(Some(sid), *cursor);
        // 这一批里出现了裁决卡就就地作答（工具级确认发生在生成中，不能等这一次生成收尾）。
        let mut carded = false;
        for l in &lines {
            render(&l.events);
            carded |= l
                .events
                .iter()
                .any(|e| matches!(e, SessionEvent::DecisionCard { .. }));
        }
        *cursor = head;
        if carded {
            answer_gates(ops, sid, cursor);
        }
        match rx.try_recv() {
            Ok(r) => return r,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("生成线程异常结束".to_string())
            }
        }
    }
}

/// 目的：登记处为空时的引导文案（**CLI 的说法**：它不再直接点模块）。
pub(crate) const NO_AGENTS: &str = "登记处还没有 agent：请先到 Web 界面「设置 → agent 管理」建一个";

/// 目的：把用户输入的名字串切成名字列表（逗号 / 中文逗号 / 空白分隔）。 这是**传输侧**的解析（argv 怎么分隔是 CLI 的事），不是业务规则。
pub(crate) fn split_names(arg: &str) -> Vec<String> {
    arg.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 目的：点名：按名字取登记处存档 → 用例输入；空登记处给引导（文案见 `NO_AGENTS`）。
pub(crate) fn pick_agents(ops: &Ops, names: &[String]) -> Result<Vec<AgentInstance>, String> {
    if ops.registry.agents()?.is_empty() {
        return Err(NO_AGENTS.to_string());
    }
    let views = ops.registry.pick_agents(names)?;
    Ok(views.iter().map(AgentInstance::from_view).collect())
}

/// 目的：虚拟机档的虚拟根怎么选——本机只有一个装齐的就自动用它；多个（或一个都没装齐）返回 None，由调用方如实说明。
pub(super) fn pick_guest(
    guests: &[crate::capabilities::provision::api::GuestBase],
) -> Option<String> {
    let ready: Vec<&crate::capabilities::provision::api::GuestBase> =
        guests.iter().filter(|g| g.ready).collect();
    match ready.as_slice() {
        [one] => Some(one.guest.clone()),
        many if many.len() > 1 => {
            let names: Vec<&str> = many.iter().map(|g| g.guest.as_str()).collect();
            println!(
                "[提示] 本机有多个虚拟根可选（{}）：这里先用第一个",
                names.join("、")
            );
            Some(names[0].to_string())
        }
        _ => None,
    }
}

/// 目的：建会话走**动作表**——与 Web / agent 同一份声明、同一处授权，CLI 只把用户的选择变成参数。
pub(crate) fn create_session_action(
    ops: &Ops,
    name: &str,
    mode: &str,
    agents: &[AgentInstance],
    task: Option<&str>,
    tier: Tier,
) -> Result<String, String> {
    let agents: Vec<serde_json::Value> = agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "transient": a.transient,
                "modules": a.modules,
                "model": a.model,
                "chara": a.chara,
            })
        })
        .collect();
    let mut args = serde_json::json!({
        "name": name,
        "mode": mode,
        "agents": agents,
        "tier": tier.as_str(),
    });
    if let Some(t) = task {
        args["task"] = serde_json::json!(t);
    }
    // 虚拟机档要选一个虚拟根（人默认是设置里的档位）：本机只有一个就自动用它，
    // 多个就如实说清有哪些（CLI 不代人挑），一个都没装就交给后端给"怎么补"。
    if tier == Tier::Vm {
        // 读不到清单（比如补齐面没接入）时不给 guest：交给后端按同一份清单如实拒绝。
        if let Ok(t) = ops.core.tier_choices() {
            match pick_guest(&t.guests) {
                Some(g) => args["guest"] = serde_json::json!(g),
                None => {
                    return Err(
                        "虚拟机档要选一个虚拟根：跑 solomni --provision --ensure --yes 装好之后会列出可选的那些"
                            .to_string(),
                    )
                }
            }
        }
    }
    match ops.actions.act(ActionCall {
        id: "create_session".to_string(),
        args,
        caller: Caller::User,
        out: Output::Final,
    })? {
        Acted::Done(v) => v
            .get("sid")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "建会话回包缺 sid".to_string()),
        _ => Err("建会话该给结构化结果".to_string()),
    }
}
