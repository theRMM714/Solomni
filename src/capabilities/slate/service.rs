//! 拟名单的**用例实现**：请核心报一份名单 → 按登记处与工作区核验 → 按模式收束。
//!
//! 它驱动 IO（一次模型调用 + 只读核实回路），所以不在 `domain/`；纯规则（模式收束）在
//! `domain/proposal.rs`。协议只有这一处：两个调用方（推荐 / 代拟）走同一条 `slate` 工具载荷。

use crate::capabilities::llm::api::Msg;
use crate::capabilities::prompt::api::Segment;
use crate::capabilities::registry::api::{model_listing, resolve_picks, RosterPick};
use crate::capabilities::session::api::core_operation;
use crate::capabilities::slate::api::{Mode, Parties, Pick, Proposal, Request};
use crate::capabilities::slate::domain::proposal::collapse;

/// 拟一份名单：让核心按本次需求提出候选，按登记处与工作区现状核验成**可直接开工**的名单。
///
/// 前置判据（登记处有没有模型、核心默认设没设）**不在这里**：那是调用方对用户的提示
///（怎么修），与名单本身的合法性无关——空登记处会让每个组装项因「模型不存在」被逐条拒收。
pub fn propose(parties: &Parties<'_>, req: &mut Request<'_>) -> Result<Proposal, String> {
    // 形态描述文案在提示词册里（代码不硬编码给模型的说明）。
    let mode_text = match req.mode {
        Mode::Single => parties.prompt.text(Segment::SlateModeSingle).to_string(),
        Mode::Collab => parties.prompt.text(Segment::SlateModeCollab).to_string(),
    };
    let user = parties.prompt.render(
        Segment::SlateUser,
        &[
            ("mode", mode_text),
            (
                "agents",
                crate::capabilities::registry::api::listing(parties.prompt, parties.agents),
            ),
            (
                "modules",
                crate::capabilities::workspace::api::listing(
                    parties.roster,
                    &parties.prompt.tools(),
                ),
            ),
            (
                "models",
                model_listing(parties.models, &parties.prompt.tools()),
            ),
            ("task", req.task.to_string()),
        ],
    );
    // 核心操作走工具调用：名单由 slate 工具承载（推荐与代拟共用的那一条协议）。
    let payload = core_operation(
        parties.tools,
        "planner",
        "slate",
        req.tool_mode,
        req.chat,
        &[
            Msg::system(parties.prompt.text(Segment::SlateSystem).to_string()),
            Msg::user(user),
        ],
        req.opts,
        req.cancel,
        req.verify.as_deref_mut(),
        req.sink,
    )?;
    // 载荷里就是名单**数组**本身（工具参数 picks 的值）。
    let parsed: Vec<RosterPick> = payload
        .get("picks")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or_else(|| {
            format!(
                "拟名单的载荷没有 picks 数组：{}",
                payload.to_string().chars().take(200).collect::<String>()
            )
        })?;
    // 逐条核验（存在性、模型真实、整份名单内模块不重复）；拒收项交回调用方如实告知。
    let (picks, rejected) = resolve_picks(parsed, parties.agents, parties.roster, parties.models);
    let picks: Vec<Pick> = picks
        .into_iter()
        .map(|(agent, why)| Pick { agent, why })
        .collect();
    Ok(Proposal {
        picks: collapse(picks, req.mode),
        rejected,
    })
}
