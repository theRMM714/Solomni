//! 目的：终端渲染：花名册、菜单、事件流与模型/供应商标签。
//! 管：把能力面的事实与事件词汇翻成终端行文。
//! 不管：命令解析、交互裁决、会话流。
//! 联动：`cli.rs`、`cli/gate.rs`、`docs/presentation/contracts.md`

use super::*;

/// 目的：启动页副行：终端转录中心、版本与根路径。
pub(super) fn startup_tagline() -> String {
    let root = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    format!("终端转录中心 · v{} · {}", env!("CARGO_PKG_VERSION"), root)
}

/// 目的：启动页事实行：agent 数、模块数、档位与核心默认模型各一格。
pub(super) fn startup_facts(ops: &Ops) -> Vec<String> {
    let agents = ops.registry.agents().map(|a| a.len()).unwrap_or(0);
    let modules = ops.modules.roster().map(|r| r.modules.len()).unwrap_or(0);
    let tier = ops
        .registry
        .settings()
        .map(|s| s.tier.as_str())
        .unwrap_or("host");
    let model = ops.registry.core_model().ok().flatten();
    vec![
        fact("agent", &agents.to_string()),
        fact("模块", &modules.to_string()),
        fact("档位", tier),
        fact("模型", model.as_deref().unwrap_or("（未设定）")),
    ]
}

/// 目的：状态行：就绪、核心默认模型与档位。
pub(super) fn startup_status(ops: &Ops) -> String {
    let model = ops.registry.core_model().ok().flatten();
    let tier = ops
        .registry
        .settings()
        .map(|s| s.tier.as_str())
        .unwrap_or("host");
    format!(
        "● 就绪 · 模型 {} · 档位 {}",
        model.as_deref().unwrap_or("（未设定）"),
        tier
    )
}

fn fact(label: &str, value: &str) -> String {
    format!(
        "{} {}",
        term::ink(theme::Ink::Dim, label),
        term::bold(theme::Ink::Fg2, value)
    )
}

pub(super) fn print_roster(ops: &Ops) {
    let roster = match ops.modules.roster() {
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
    let hard: Vec<crate::capabilities::conductor::api::RuntimeDiagnosis> = report
        .diagnoses
        .iter()
        .filter(|d| {
            !matches!(
                d,
                crate::capabilities::conductor::api::RuntimeDiagnosis::Package(
                    crate::capabilities::packages::api::Problem::Missing { .. }
                )
            )
        })
        .cloned()
        .collect();
    if !hard.is_empty() {
        println!(
            "[档位诊断] {}（虚拟机档要先解决这些才能开始会话）",
            crate::capabilities::conductor::api::diagnose_text(&hard)
        );
    }
    for r in &report.rejected_packages {
        println!("[运行包拒收] {}", r);
    }
}

pub(super) fn print_menu(ops: &Ops) {
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
    println!(
        "命令：single [agent名…] | collab [agent名…|?] | proxy（决定权整块交给核心） | module [模块id.工具名 [json]]（不经 AI 直接用模块工具） | provider list|add|rm|discover | model list|add|rm | core <模型id> | rewind <会话> archive|delete <行id> | rewind <会话> restore <标记id> | work <会话> log|show|restore <提交号> | resident list|start|stop|enable|disable|call <模块id> <服务名> [操作名] [json] | secret | diagnose <会话> | rescan | webui | exit"
    );
}

/// 目的：模型标签（CLI 展示文案；核心默认是登记处的概念，不是提示词）。
pub(super) fn model_label(model: Option<&str>) -> String {
    model
        .map(|m| m.to_string())
        .unwrap_or_else(|| "（核心默认）".to_string())
}

/// 目的：展示文案归呈现层：conductor 只给结构化事实（视图），怎么排版是这里的事。
pub(super) fn provider_line(p: &ProviderView) -> String {
    format!("{}  {}", p.id, p.base_url)
}

pub(super) fn model_line(m: &ModelView, core: Option<&str>) -> String {
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

/// 目的：事件渲染入口：交互终端走富渲染，非 TTY 走原样文本（脚本口径逐字不变）。
pub(super) fn render(events: &[SessionEvent]) {
    if term::tty() {
        term::live_clear();
        render_rich(events);
    } else {
        render_plain(events);
    }
}

fn head(glyph: &str, ink: theme::Ink, name: &str, verb: &str) -> String {
    let mut out = format!("{} {}", term::ink(ink, glyph), term::bold(ink, name));
    if !verb.is_empty() {
        out.push(' ');
        out.push_str(&term::ink(theme::Ink::Dim, verb));
    }
    out
}

fn render_rich(events: &[SessionEvent]) {
    for e in events {
        match e {
            SessionEvent::Notice(n) => {
                println!(
                    "{} {}",
                    term::ink(theme::Ink::Dim, "·"),
                    term::ink(theme::Ink::Dim, n)
                )
            }
            SessionEvent::Transcript(lines) => render_lines(lines),
            SessionEvent::Working { agent } => {
                if let Some(a) = agent {
                    println!(
                        "{} {}",
                        term::ink(theme::Ink::Ice2, "●"),
                        term::ink(theme::Ink::Dim, &format!("{} 开始干活，稍等", a))
                    );
                }
            }
            SessionEvent::Plan(p) => {
                println!("{}", head("▸", theme::Ink::Ice2, "整理", ""));
                for row in p.lines() {
                    println!("  {}", row);
                }
            }
            SessionEvent::PlanReview { plan, chain } => {
                println!("{}", head("◈", theme::Ink::Amber, "待审", "同意之后才开工"));
                for row in plan.lines() {
                    println!("  {}", row);
                }
                for n in &chain.nodes {
                    println!(
                        "  {} {} {}",
                        term::ink(theme::Ink::Dim, &format!("[{}]", n.id)),
                        term::bold(theme::Ink::Fg2, &n.title),
                        term::ink(theme::Ink::Dim, &format!("→ {}", n.assignee))
                    );
                }
            }
            SessionEvent::NodeStarted {
                node,
                sid,
                assignee,
            } => println!(
                "{} {} {}",
                term::ink(theme::Ink::Ice2, "→"),
                term::bold(theme::Ink::Fg2, &format!("节点 {}", node)),
                term::ink(
                    theme::Ink::Dim,
                    &format!("子会话 {} · 负责人 {}", sid, assignee)
                )
            ),
            SessionEvent::Report { id, text, rework } => {
                let verb = if *rework > 0 {
                    format!("返工第 {} 轮", rework)
                } else {
                    String::new()
                };
                println!("{}", head("◆", theme::Ink::Ice2, id, &verb));
                for row in text.lines() {
                    println!("  {}", row);
                }
            }
            SessionEvent::Review { items, raw } => {
                println!("{}", head("◈", theme::Ink::Ice2, "验收", ""));
                if items.is_empty() {
                    println!(
                        "  {}",
                        term::ink(theme::Ink::Dim, "（清单解析失败，原文如下）")
                    );
                    for row in raw.lines() {
                        println!("  {}", term::ink(theme::Ink::Dim, row));
                    }
                }
                for i in items {
                    let (glyph, ink) = check_mark(&i.status);
                    println!(
                        "  {} {} {}",
                        term::ink(ink, glyph),
                        i.item,
                        term::ink(theme::Ink::Dim, &i.note)
                    );
                }
            }
            SessionEvent::Delivery { ok, over_rework } => {
                let (glyph, ink, text) = if *ok {
                    ("✓", theme::Ink::Green, "全部通过，交付用户。")
                } else if *over_rework {
                    ("◈", theme::Ink::Amber, "返工超限仍未通过，交用户裁决。")
                } else {
                    ("✗", theme::Ink::Red, "未能通过，详见上面的验收清单。")
                };
                println!("{} {}", term::ink(ink, glyph), term::ink(ink, text));
            }
            SessionEvent::DecisionCard { card, .. } => println!(
                "{} {} {}",
                term::ink(theme::Ink::Amber, "◈"),
                term::bold(theme::Ink::Amber, &card.message.title),
                term::ink(
                    theme::Ink::Dim,
                    &format!("（{} 在等你作答）", card.envelope.name)
                )
            ),
            SessionEvent::DecisionAnswer(a) => println!(
                "{} {}",
                term::ink(theme::Ink::Dim, "·"),
                term::ink(
                    theme::Ink::Dim,
                    &format!("{} 答了 {}：选了 {}", a.by, a.card, a.option)
                )
            ),
            SessionEvent::DecisionVoid { cards, reason } => println!(
                "{} {}",
                term::ink(theme::Ink::Dim, "·"),
                term::ink(
                    theme::Ink::Dim,
                    &format!(
                        "作废 {} 张没答的卡（{}）：{}",
                        cards.len(),
                        cards.join("、"),
                        reason
                    )
                )
            ),
            SessionEvent::Usage { usage, .. } => term::status(&format!(
                "● 上下文 {}/{}（{}）",
                tokens(usage.total_tokens),
                tokens(usage.window),
                if usage.exact { "实计" } else { "估算" }
            )),
            // 短暂事件（流式增量 / 工具调用）进的是同一份事件的实时副本：转录行随后就到，这里不重复打。
            SessionEvent::Delta { .. } | SessionEvent::ToolCall(_) => {}
            SessionEvent::DiscussionDone { .. }
            | SessionEvent::Ended
            | SessionEvent::Rewound { .. } => {}
        }
    }
}

fn check_mark(status: &str) -> (&'static str, theme::Ink) {
    let s = status.to_ascii_lowercase();
    if s.contains("pass") || s.contains("ok") || s.contains("done") || s.contains("通过") {
        ("✓", theme::Ink::Green)
    } else if s.contains("fail") || s.contains("未通过") || s.contains("错误") {
        ("✗", theme::Ink::Red)
    } else {
        ("·", theme::Ink::Dim)
    }
}

fn render_lines(lines: &[crate::capabilities::session::api::LineView]) {
    for l in lines {
        if l.kind == "compacted" {
            let up_to = l
                .payload
                .as_ref()
                .and_then(|p| p.get("up_to"))
                .and_then(|u| u.as_u64())
                .unwrap_or(0);
            println!(
                "{} {}",
                term::ink(theme::Ink::Dim, "[压缩]"),
                term::ink(
                    theme::Ink::Dim,
                    &format!("第 {} 行之前已压成摘要；此后原样保留（仍可查看）：", up_to)
                )
            );
            for row in l.line.lines() {
                println!("  {}", term::ink(theme::Ink::Dim, row));
            }
            continue;
        }
        if l.kind == "tool" {
            if let Some(t) = &l.tool {
                print_tool(t);
            }
            continue;
        }
        if l.kind == "user" || l.speaker == "用户" {
            println!(
                "{} {}",
                term::ink(theme::Ink::Ice, "›"),
                term::bold(theme::Ink::Ice, &l.line)
            );
            continue;
        }
        if l.kind == "round" {
            println!(
                "{} {}",
                term::ink(theme::Ink::Line, "────"),
                term::ink(theme::Ink::Dim, &l.line)
            );
            continue;
        }
        if l.system || l.kind == "system" {
            for row in l.line.lines() {
                println!(
                    "{} {}",
                    term::ink(theme::Ink::Dim, "·"),
                    term::ink(theme::Ink::Dim, row)
                );
            }
            continue;
        }
        if l.speaker.is_empty() {
            for row in l.line.lines() {
                println!("  {}", row);
            }
            continue;
        }
        println!("{}", head("◆", theme::Ink::Ice2, &l.speaker, &l.verb));
        for row in l.line.lines() {
            println!("  {}", row);
        }
    }
}

fn print_tool(t: &crate::capabilities::session::api::ToolCallView) {
    let (glyph, ink) = if t.ok {
        ("✓", theme::Ink::Green)
    } else {
        ("✗", theme::Ink::Red)
    };
    let label = t.label();
    let pad = 18usize.saturating_sub(width::cols(&label));
    println!(
        "{} {}{} {} {}",
        term::ink(theme::Ink::Ice2, "⏺"),
        term::bold(theme::Ink::Fg2, &label),
        " ".repeat(pad),
        term::ink(theme::Ink::Dim, &arg_preview(&t.args)),
        term::ink(ink, glyph)
    );
}

fn arg_preview(args: &str) -> String {
    let one: String = args.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        return String::new();
    }
    width::cut(&one, 64)
}

fn tokens(n: u64) -> String {
    if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

pub(super) fn render_plain(events: &[SessionEvent]) {
    for e in events {
        match e {
            SessionEvent::Notice(n) => println!("{}", n),
            SessionEvent::Transcript(lines) => {
                println!("---- 转录 ----");
                for l in lines {
                    if l.kind == "compacted" {
                        let up_to = l
                            .payload
                            .as_ref()
                            .and_then(|p| p.get("up_to"))
                            .and_then(|u| u.as_u64())
                            .unwrap_or(0);
                        println!(
                            "[压缩] 第 {} 行之前已压成摘要；此后原样保留（仍可查看）：\n{}",
                            up_to, l.line
                        );
                        continue;
                    }
                    // 系统消息（提醒/"未回应"这类**不是谁说的**内容）标出来，别和用户/发言混在一起。
                    let mark = if l.system { "[系统] " } else { "" };
                    println!("{}{}", mark, l.line);
                }
            }
            SessionEvent::DiscussionDone { .. } => {}
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
            | SessionEvent::ToolCall(_)
            | SessionEvent::Rewound { .. } => {}
            // 上下文用量是状态栏事实：Web 有状态栏，终端不逐次打。
            SessionEvent::Usage { .. } => {}
        }
    }
}
