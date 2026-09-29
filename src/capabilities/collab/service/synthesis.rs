//! **整理与审查**：核心整理讨论出方案与任务链（`synthesize`）+ 总验收清单与返工判定（`Execution` / `CheckItem`）。
//!
//! 载荷都从**工具调用**取（plan / checklist），解析失败如实报错、不猜。
//!
//! 状态机在 `discussion`；本文件放整理那一段与验收清单。

use super::discussion::*;
use crate::capabilities::llm::api::{Chat, Msg};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::session::api::MemberTools;
use crate::capabilities::session::api::SessionEvent;
use serde::Deserialize;
use std::collections::BTreeMap;

impl Discussion {
    /// 全员同意后：核心整理——总结讨论，为每个留下的成员写执行任务提示词。
    /// 整理：核心 AI 总结讨论并出**任务链**（见 docs/collab/task-chain.md）。
    /// 产出走 **plan 工具调用**（核心操作不手写 JSON）；没调用或载荷不合法就**如实报错**。
    pub fn synthesize(
        &self,
        core_chat: &mut dyn Chat,
        mode: crate::capabilities::llm::api::ToolMode,
        verify: Option<&mut MemberTools>,
        // 核心这一轮的行（工具行 + 思维链 + 正文）推给谁：落不落由那个会话模块定。
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<
        (
            String,
            crate::capabilities::taskchain::api::TaskChain,
            String,
        ),
        String,
    > {
        let roster = self
            .members
            .iter()
            .filter(|m| m.present)
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
            .join("、");
        let user = self.prompts.render(
            Segment::SynthesizeUser,
            &[
                ("roster", roster),
                (
                    "transcript",
                    self.transcript
                        .iter()
                        .map(|l| l.render())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ],
        );
        let msgs = vec![
            Msg::system(self.prompts.text(Segment::SynthesizeSystem).to_string()),
            Msg::user(user),
        ];
        if self.cancelled() {
            return Err("已停止".to_string());
        }
        // 核心操作走工具调用：载荷形状是 plan + nodes，入口是 plan 工具。
        let payload = crate::capabilities::session::api::core_operation(
            &*self.systools,
            "planner",
            "plan",
            mode,
            core_chat,
            &msgs,
            self.opts(),
            Some(&self.cancel),
            verify,
            sink,
        )?;
        if self.cancelled() {
            return Err("已停止".to_string());
        }
        let parsed: SynthReply = serde_json::from_value(payload)
            .map_err(|e| format!("plan 工具的载荷不合法（{}）", e))?;
        let chain = crate::capabilities::taskchain::api::TaskChain {
            nodes: parsed
                .nodes
                .into_iter()
                .map(|n| crate::capabilities::taskchain::api::TaskNode {
                    id: n.id,
                    title: n.title,
                    objective: n.objective,
                    assignee: n.assignee,
                    deps: n.deps,
                    status: crate::capabilities::taskchain::api::NodeStatus::Pending,
                    sub_session: None,
                    report: None,
                    acceptance: None,
                    reported: false,
                })
                .collect(),
        };
        Ok((parsed.plan, chain, parsed.advice))
    }
}

/// 核心整理的结构化回执（形状见 prompts/roles/planner.yaml 的 synthesize）。
#[derive(Debug, Clone, Deserialize)]
struct SynthReply {
    plan: String,
    #[serde(default)]
    nodes: Vec<SynthNode>,
    /// 核心给用户的**建议**（推荐现在开工还是先改方案）；没有就是空串。
    #[serde(default)]
    advice: String,
}

/// 任务链里的一个节点（核心给的是"意图"，状态与子会话由核心自己管）。
#[derive(Debug, Clone, Deserialize)]
struct SynthNode {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    objective: String,
    #[serde(default)]
    assignee: String,
    #[serde(default)]
    deps: Vec<String>,
}

/// 验收清单条目：核心输出的结构化核对结果。
#[derive(Debug, Clone, Deserialize)]
pub struct CheckItem {
    pub item: String,
    pub status: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    /// `status = fail` 时**要返工的节点 id**（核心按链校验；填错/漏填会让核心重填）。
    /// 为什么是节点 id 而不是人名：链是节点级的，一个 agent 可能负责多个节点（见 task-chain.md 五）。
    #[serde(default)]
    pub rework: Option<String>,
}

/// 执行与验收：成员按任务干活并回报；核心对照回报产出结构化清单。
pub struct Execution {
    pub reports: BTreeMap<String, String>,
    /// 验收原始输出（解析失败时如实呈现）。
    pub checklist_raw: String,
    /// 结构化清单；空 = 解析失败（all_pass 保守判否）。
    pub items: Vec<CheckItem>,
    /// 执行/验收途中调用失败（超时 / 网络）：非空 = 本轮**中断**，不交付。
    /// 上层据此如实告知用户；会话保持可继续（用户点「继续」重新推进）。
    pub error: Option<String>,
    /// 被用户「停止」：非空 = 本轮**停止**，未收完的回报**不入册**（半截回报进转录会误导验收）。
    pub stopped: bool,
    /// 「停止」标志（泵注入）：每次模型调用前与调用中途都看它。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Execution {
    pub fn new() -> Execution {
        Execution {
            reports: BTreeMap::new(),
            checklist_raw: String::new(),
            items: Vec::new(),
            error: None,
            stopped: false,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 是否已被要求停止（总验收共用）。
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    // 参数是一组必须一路透传的出口（chat / plan / 对照表 / 重填说明 / 提示词册 / 通道 / 核实环境）：
    // 与 judge_clear / review_nodes 同一取舍（见 docs/testing/quality-isolation.md）。
    #[allow(clippy::too_many_arguments)]
    /// 总验收（一次调用）。`nodes` = "节点 id — 负责人"对照表（模型只能从这里选 `rework`）；
    /// `retry` = 上一次填错了要它重填的话（核心据此**一直重填**到合法为止，不设次数上限）。
    pub fn review(
        &mut self,
        core_chat: &mut dyn Chat,
        plan: &str,
        nodes: &str,
        retry: Option<&str>,
        prompt: &dyn Prompt,
        systools: &dyn crate::capabilities::tools::api::Tools,
        llm: crate::capabilities::llm::api::LlmOpts,
        mode: crate::capabilities::llm::api::ToolMode,
        verify: Option<&mut MemberTools>,
        // 核心这一轮的行推给谁（总验收也要能看到它在核对什么）。
        sink: &mut dyn FnMut(SessionEvent),
    ) {
        let reports = self
            .reports
            .iter()
            .map(|(id, r)| format!("[{}] {}\n", id, r))
            .collect::<Vec<_>>()
            .join("");
        let user = prompt.render(
            Segment::ReviewUser,
            &[
                ("plan", plan.to_string()),
                ("reports", reports),
                ("nodes", nodes.to_string()),
            ],
        );
        let mut msgs = vec![
            Msg::system(prompt.text(Segment::ReviewSystem).to_string()),
            Msg::user(user),
        ];
        if let Some(again) = retry {
            msgs.push(Msg::user(again.to_string()));
        }
        if self.cancelled() {
            self.stopped = true;
            return;
        }
        let opts = crate::capabilities::llm::api::CompleteOpts::plain(llm.stream)
            .with_timeout(llm.timeout_secs);
        // 核心操作走工具调用：总验收清单由 checklist 工具承载。
        let made = crate::capabilities::session::api::core_operation(
            systools,
            "orchestrator",
            "checklist",
            mode,
            core_chat,
            &msgs,
            opts,
            Some(&self.cancel),
            verify,
            sink,
        );
        if self.cancelled() {
            self.stopped = true;
            return;
        }
        match made {
            Ok(payload) => {
                // 载荷里的 items 数组；形状不对就如实记空清单（all_pass 保守判否）。
                self.items = payload
                    .get("items")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Vec<CheckItem>>(v).ok())
                    .unwrap_or_default();
                self.checklist_raw = payload.to_string();
            }
            Err(err) => {
                self.error = Some(err);
            }
        }
    }

    /// `status = fail` 且填了 `rework` 的节点（按清单顺序去重）：核心据此**只重派这些**。
    pub fn rework_targets(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for it in &self.items {
            if it.status.eq_ignore_ascii_case("fail") {
                if let Some(r) = it.rework.as_deref() {
                    if !r.trim().is_empty() && !out.iter().any(|x| x == r) {
                        out.push(r.to_string());
                    }
                }
            }
        }
        out
    }

    /// 这次判定里的问题（空 = 合法）：fail 必须填 `rework`，且只能是 `known` 里的节点 id。
    /// 核心据此**要求模型重填**（一直重填，不设上限）——绝不静默丢掉一条判定。
    pub fn rework_problems(&self, known: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for it in &self.items {
            if !it.status.eq_ignore_ascii_case("fail") {
                continue;
            }
            match it.rework.as_deref().map(str::trim) {
                Some(r) if !r.is_empty() => {
                    if !known.iter().any(|k| k == r) {
                        out.push(format!("条目「{}」填的 rework={} 不在节点表里", it.item, r));
                    }
                }
                _ => out.push(format!("条目「{}」没过（fail）但没填 rework", it.item)),
            }
        }
        out
    }

    pub fn all_pass(&self) -> bool {
        // 清单为空（解析失败）= 保守判否；有清单则逐项全过才通过。
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|i| i.status.eq_ignore_ascii_case("pass"))
    }
}
