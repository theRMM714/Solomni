//! 终端转录中心：解析命令 → 用入站能力面 → 渲染事件流。
//! 只做解析与渲染，不做业务决策；Web 前端与它并列，共用同一能力面与事件词汇。

use crate::capabilities::conductor::api::{Acted, ActionCall, Caller};
use crate::capabilities::conductor::api::{
    AgentInstance, DecisionCard, DecisionWaiter, SessionEvent, Tier,
};
use crate::capabilities::conductor::api::{Ops, Output};
use crate::capabilities::registry::api::{ModelView, ProviderView};
use std::io::Write;

/// 离开转录中心时的去向：退出，或转入 Web 转录中心（端口）。
pub enum CliExit {
    Exit,
    Web(u16),
}

pub fn run(ops: Ops, web_default_port: u16) -> CliExit {
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
            "proxy" => proxy_flow(&ops),
            "provider" => provider_flow(&ops, &arg),
            "model" => model_flow(&ops, &arg),
            "core" => core_flow(&ops, &arg),
            // 回档：留档（标记+折叠，可恢复）/ 删除（真的截掉）/ 恢复（删掉该标记及其后）。
            "rewind" => rewind_cmd(&ops, &arg),
            // 直接用模块工具（不经 AI）：清单与动作 id 都来自核心的动作目录。
            "module" => module_cmd(&ops, &arg),
            "rescan" => print_roster(&ops),
            // 转入 Web 转录中心：接受 webui / -webUI（启动参数也这么写），可选端口。
            "webui" | "-webui" | "web" | "-web" => {
                let port = arg.parse::<u16>().unwrap_or(web_default_port);
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

/// 目的：把用户写的模块工具名规整成动作 id——`<模块id>.<工具名>` 与完整的 `module.<模块id>.<工具名>` 都收。
pub(crate) fn module_action_id(arg: &str) -> String {
    let a = arg.trim();
    if a.starts_with("module.") {
        a.to_string()
    } else {
        format!("module.{}", a)
    }
}

/// 直接用模块工具（不经 AI）：`module` 列清单，`module <模块id>.<工具名> [json 参数]` 跑一次。
/// 清单来自核心的动作目录（`module.<模块id>.<工具名>`），所以与 Web 看到的是同一份事实。
fn module_cmd(ops: &Ops, arg: &str) {
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
        if modules.is_empty() {
            println!("（没有声明 tools 的模块；模块可以只写 system，不声明外部工具）");
        }
        for a in &modules {
            println!("  {:<28} {}", a.id, first_line(&a.desc));
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

/// 命令行回档：给共享区与整棵子树都对齐到同一个点。
/// 留档 = 标记 + 折叠（可恢复）；删除 = 真的截掉；恢复 = 删掉该标记及其后（不可恢复）。
fn rewind_cmd(ops: &Ops, arg: &str) {
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

fn print_roster(ops: &Ops) {
    let roster = match ops.workspace.roster() {
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
    let report = match ops.core.runtime_report(tier) {
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
    let hard: Vec<crate::capabilities::workspace::api::Diagnosis> = report
        .diagnoses
        .iter()
        .filter(|d| {
            !matches!(
                d,
                crate::capabilities::workspace::api::Diagnosis::Missing { .. }
            )
        })
        .cloned()
        .collect();
    if !hard.is_empty() {
        println!(
            "[档位诊断] {}（虚拟机档要先解决这些才能开始会话）",
            crate::capabilities::workspace::api::diagnose_text(&hard)
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
    println!("命令：single [agent名…] | collab [agent名…|?] | proxy（决定权整块交给核心） | module [模块id.工具名 [json]]（不经 AI 直接用模块工具） | provider list|add|rm|discover | model list|add|rm | core <模型id> | rewind <会话> archive|delete <行id> | rewind <会话> restore <标记id> | rescan | webui | exit");
}

/// 模型标签（CLI 展示文案；核心默认是登记处的概念，不是提示词）。
fn model_label(model: Option<&str>) -> String {
    model
        .map(|m| m.to_string())
        .unwrap_or_else(|| "（核心默认）".to_string())
}

/// 展示文案归呈现层：conductor 只给结构化事实（视图），怎么排版是这里的事。
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
            // **裁决卡**：界面只认这四个字段（信封 / 消息 / 选项），不认识业务含义。
            // 交互模式下随后由 `answer_gates` 把它整张打出来并就地作答；这里只提一句，不重复整张。
            SessionEvent::DecisionCard { card, .. } => {
                println!(
                    "[裁决] 有一张卡在等你：{}（{}）——按提示作答",
                    card.message.title, card.envelope.name
                )
            }
            // 一次回答的记录（谁答的、选了哪个 id）：如实打出来，便于对账。
            SessionEvent::DecisionAnswer(a) => {
                println!("[裁决] {} 答了 {}：选了 {}", a.by, a.card, a.option)
            }
            // 整队作废（停止 / 关闭）：如实说清作废了哪几张、为什么——等待方按"拒绝"解开。
            SessionEvent::DecisionVoid { cards, reason } => println!(
                "[裁决] 作废 {} 张没答的卡（{}）：{}",
                cards.len(),
                cards.join("、"),
                reason
            ),
            // 流式增量与工具调用实时事件都是短暂事件，终端不在流中渲染（最终行会到）。
            // 短暂事件（流式增量 / 运行态 / 工具调用）：Web 前端用来做实时渲染，CLI 不逐条打。
            SessionEvent::Delta { .. }
            | SessionEvent::Working { .. }
            | SessionEvent::ToolCall(_) => {}
        }
    }
}

/// **订阅事件台**：把 `from` 之后属于这条会话的批次渲染出来，返回新的游标。
/// 命令回包只给头部序号——事实一条不落都在事件台上，CLI 与 Web 前端读的是同一份。
fn drain(ops: &Ops, sid: &str, from: u64) -> u64 {
    let (lines, head, _oldest) = ops.events.snapshot(Some(sid), from);
    for l in &lines {
        render(&l.events);
    }
    head
}

/// 一次命令之后的订阅。回档/改需求给的是**重放快照**（不是增量事实）：终端不重复打，
/// 只把游标推到当前头部（不重渲已有的行）。
fn follow(ops: &Ops, sid: &str, cursor: &mut u64, acted: Acted) {
    if matches!(acted, Acted::Replayed(_)) {
        *cursor = ops.events.head();
        return;
    }
    *cursor = drain(ops, sid, *cursor);
}

/// CLI 侧的动作（拥有字符串）：生成要放后台线程，所以不能借用调用栈上的 `&str`。
enum CliAction {
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

/// 把还在等的那几张如实打出来（**只有队首能答**）。
fn print_waiting(waiting: &[DecisionWaiter]) {
    for line in waiting_lines(waiting) {
        println!("{}", line);
    }
}

/// 把一张裁决卡按它的四个字段打出来（信封 / 消息 / 选项）：CLI 不认识业务含义，只认这几格。
fn print_card(card: &DecisionCard) {
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

/// 选中的选项 id：先按序号、再按 id 原文认——两者都是那张卡上**用户看得见**的东西。
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

/// 裁决门：**按卡片上的选项作答**（回答回的是选项 id + 附言，不是自由文本）。
/// 不点不继续：空输入 = 先不答（会话仍在等）。
fn answer_gates(ops: &Ops, sid: &str, cursor: &mut u64) {
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

/// 回答之后**跟到这一段收尾**再回提示符：回答是直路（先落定、处置脱离调用方跑），
/// 不跟的话终端会在讨论还在跑的时候就回到提示符——用户既看不到后续，也等不到下一张卡。
/// 判据：这条会话不在跑，且事件台连着几拍没有再长（等的是"脱离调用方那一段"，不是某一次生成）。
fn wait_quiet(ops: &Ops, sid: &str, cursor: &mut u64) {
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

/// 一次生成放**后台线程**；主线程一边渲染事件、一边就地按卡作答（工具级确认等在工作线程上）。
/// 为什么：回答要在生成进行中读键盘，同步调用会把主线程堵在生成里，读不到输入。
fn act_interactive(
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

/// 登记处为空时的引导文案（**CLI 的说法**：它不再直接点模块）。
pub(crate) const NO_AGENTS: &str = "登记处还没有 agent：请先到 Web 界面「设置 → agent 管理」建一个";

/// 把用户输入的名字串切成名字列表（逗号 / 中文逗号 / 空白分隔）。
/// 这是**传输侧**的解析（argv 怎么分隔是 CLI 的事），不是业务规则。
pub(crate) fn split_names(arg: &str) -> Vec<String> {
    arg.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 点名：按名字取登记处存档 → 用例输入；空登记处给引导（文案见 `NO_AGENTS`）。
pub(crate) fn pick_agents(ops: &Ops, names: &[String]) -> Result<Vec<AgentInstance>, String> {
    if ops.registry.agents()?.is_empty() {
        return Err(NO_AGENTS.to_string());
    }
    let views = ops.registry.pick_agents(names)?;
    Ok(views.iter().map(AgentInstance::from_view).collect())
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

// ---------- 形态一：单 agent（模块数不限） ----------

fn single_flow(ops: &Ops, arg: &str) {
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

// ---------- 模式三：代理（决定权整块交给核心） ----------

/// 把决定权整块交给核心：**没有名单**（核心自己挑人、建子工作），接着就是跟它对话。
/// 选这一形态本身就是**授予全权**（`WorkMode::Proxy` → `meta.delegation`）。
fn proxy_flow(ops: &Ops) {
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

// ---------- 模式四：协作（按裁决卡的选项作答驱动） ----------

fn collab_flow(ops: &Ops, arg: &str) {
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

/// 目的：CLI 读输入读到尽头（EOF）时，把挂着的裁决按**停止 = 拒绝**收场——前端负责解开等待方。
/// 返回：真的停下了这条（整棵）会话。
/// 约束：停止 = 拒绝（整队作废）；不套用发起方声明的默认项——那会把"没人答"变成放行。
pub(crate) fn eof_stop(ops: &Ops, sid: &str) -> bool {
    println!("[提示] 输入已到尽头：挂着的裁决按「停止 = 拒绝」收场，这一趟不继续。");
    !ops.sessions.stop(sid).is_empty()
}

/// 目的：读一行输入；**读到尽头（EOF）给 `None`**——"用户不在键盘前"与"用户先不答"是两件事。
fn prompt_opt(text: &str) -> Option<String> {
    print!("{} ", text);
    std::io::stdout().flush().ok();
    let mut s = String::new();
    match std::io::stdin().read_line(&mut s) {
        Ok(0) => None,
        Ok(_) => Some(s.trim().to_string()),
        Err(_) => None,
    }
}

fn prompt(text: &str) -> String {
    prompt_opt(text).unwrap_or_default()
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
