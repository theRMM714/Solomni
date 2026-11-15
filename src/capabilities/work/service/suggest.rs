//! 目的：拟名单入口——按本次需求推荐 agent 名单（拟名单业务的用例）。
//! 管：前置判据（登记处有没有模型、核心默认设没设）、队列分发与行解析。
//! 不管：名单的业务规则（slate 能力）、协作链推进（`conductor` 的 flow）。
//! 联动：`conductor` 的推荐接口经这里调用。

use super::*;

/// 目的：核心推荐（`suggest_models`）的系统会话 sid：一次性的推荐动作**没有工作区**，
///  它的行只推给在场的前端看（推荐是弹窗里的一次问答），不落盘，也不进会话列表。
pub const SYSTEM_SID_SUGGEST: &str = "#suggest";

impl Work {
    /// 目的：核心推荐：按本次需求推荐 agent 名单（**拟名单业务**的用例；这里只做队列分发、把行收出来）。
    ///  前置判据（登记处有没有模型 / 核心默认设没设）放在这里：它是对用户的提示，不是名单的合法性。
    pub fn suggest_models(
        &self,
        registry: &dyn Registry,
        task: &str,
        mode: WorkMode,
    ) -> Result<(Vec<AgentSuggestion>, Vec<SessionEvent>), String> {
        if !registry.any_models() {
            return Err("登记处还没有任何模型，请先到「模型登记」添加".to_string());
        }
        let channel = registry.core_channel()?;
        let roster = self.scan();
        let mut chat = self.llm.core_channel(&channel);
        // 登记处事实：**自持一份快照**（拟名单只读；一次用户动作、代价可忽略，
        // 与协作会话自持一份同义——写面是 `Box<dyn Registry>`，不可能共享出去）。
        let facts = registry.snapshot();
        // 核心这一趟的行（工具行、发言行、思维链）**一律外送**：核心没有会话、写不了盘，
        // 它只把行交出来；推到哪个 sid、落不落盘由调用方按会话种类定（这里给系统会话）。
        let mut rows: Vec<SessionEvent> = Vec::new();
        let proposal = crate::capabilities::slate::api::propose(
            &crate::capabilities::slate::api::Parties {
                prompt: &*self.prompt,
                tools: &*self.systools,
                roster: &roster,
                agents: &facts.agents,
                models: &facts.models,
            },
            &mut crate::capabilities::slate::api::Request {
                task,
                mode: match mode {
                    WorkMode::Single => crate::capabilities::slate::api::Mode::Single,
                    WorkMode::Collab => crate::capabilities::slate::api::Mode::Collab,
                    // 代理形态没有名单可拟（核心自己挑人）；如实拒绝，不拿单模式的清单糊弄。
                    WorkMode::Proxy => {
                        return Err(
                            "代理形态没有名单：决定权整块交给核心，由它自己挑人".to_string()
                        );
                    }
                },
                chat: chat.as_mut(),
                tool_mode: registry.tool_mode(None),
                opts: crate::capabilities::llm::api::CompleteOpts::plain(false),
                cancel: None,
                // 推荐是**一次性建议**（用户点了才生成、没有工作区可核实）：不接核实回路。
                verify: None,
                sink: &mut |e: SessionEvent| rows.push(e),
            },
        )?;
        for r in &proposal.rejected {
            self.log
                .warn("work::suggest_models", &format!("拟名单条目拒收：{}", r));
        }
        // 核心只建议、不代选：名单里的每一项都逐条核验过（存在性 / 模型真实 / 模块不重复）。
        let out: Vec<AgentSuggestion> = proposal
            .picks
            .into_iter()
            .map(|p| AgentSuggestion {
                reuse: !p.agent.transient,
                name: p.agent.name,
                // 复用项没有自己的模型时，落到核心默认（前端仍可改）。
                model: p
                    .agent
                    .model
                    .or_else(|| facts.core.clone())
                    .unwrap_or_default(),
                modules: p.agent.modules,
                why: p.why,
                chara: p.agent.chara,
            })
            .collect();
        if out.is_empty() {
            return Err("核心推荐没有可用结果".to_string());
        }
        Ok((out, rows))
    }
}
