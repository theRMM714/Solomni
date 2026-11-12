//! 目的：终端渲染：花名册、菜单、事件流与模型/供应商标签。
//! 管：把能力面的事实与事件词汇翻成终端行文。
//! 不管：命令解析、交互裁决、会话流。
//! 联动：`cli.rs`、`cli/gate.rs`、`docs/presentation/contracts.md`

use super::*;

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
    println!("命令：single [agent名…] | collab [agent名…|?] | proxy（决定权整块交给核心） | module [模块id.工具名 [json]]（不经 AI 直接用模块工具） | provider list|add|rm|discover | model list|add|rm | core <模型id> | rewind <会话> archive|delete <行id> | rewind <会话> restore <标记id> | work <会话> log|show|restore <提交号> | resident list|start|stop|enable|disable|call <模块id> <服务名> [操作名] [json] | secret | diagnose <会话> | rescan | webui | exit");
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

pub(super) fn render(events: &[SessionEvent]) {
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
