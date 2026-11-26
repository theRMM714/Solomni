//! 目的：工具调用的授权端口——「这一刻准不准调」，与「在不在场」（来源各自交 id）分开。
//! 管：`Policy` 端口、默认实现 `LoopPolicy`（按流程段的抑制表判）与能力组到工具 id 的映射。
//! 不管：工具面怎么合成（`assemble`）、工具形状（`tools` 总表）、沙箱档位（`permission`）。
//! 联动：`MemberTools` 在装配时注入它（`default_policy`）；引擎每次调用前问它（`src/units/business/agentloop/engine/tools.rs`）。

use std::sync::Arc;

use crate::units::business::session_model::api::{EXEC, FS_READ, FS_WRITE, SHARED_WRITE, loop_of};

/// 目的：一次调用的授权请求：调哪个工具、由哪些来源交出、在哪个流程段的哪个角色下。
pub struct ToolRequest<'a> {
    /// 目的：要调的工具 id（本回合工具面里的一个）。
    pub tool: &'a str,
    /// 目的：这一刻把该 id 交出来的来源 id（`agent` / `module` / `loop` / `workspace` / `profile`）。
    /// 约束：服务端契约的字段——默认策略只按段判，还没有按来源分开判的策略读它。
    #[allow(dead_code)]
    pub sources: &'a [String],
    /// 目的：这一回合在哪个流程段（静止段的会话是它的静止段 id）。
    pub loop_id: &'a str,
    /// 目的：这一回合的角色 id。
    pub role: &'a str,
}

/// 目的：授权结论。
pub enum Verdict {
    /// 目的：准。
    Allow,
    /// 目的：不准，理由进工具回执（模型读到原因自己换路）。
    Deny(String),
}

/// 目的：授权端口：这一次调用准不准。
/// 约束：只回答准不准；在场与否由来源回答（端口不合并两者的判据）。
pub trait Policy: Send + Sync {
    /// 目的：判这一次调用。
    fn decide(&self, req: &ToolRequest<'_>) -> Verdict;
}

/// 目的：默认策略：流程段声明抑制哪些能力组（`Loop.deny`），这一段里的调用就不能调那组工具。
/// 约束：只按段判，不看调用者是谁；段与能力组都在数据里（`src/units/business/session_model/domain/mechanism.rs`），代码里没有逐工具的名单。
pub struct LoopPolicy;

impl Policy for LoopPolicy {
    fn decide(&self, req: &ToolRequest<'_>) -> Verdict {
        let Some(seg) = loop_of(req.loop_id) else {
            return Verdict::Allow;
        };
        for ability in seg.deny {
            if ids_of(ability).contains(&req.tool) {
                return Verdict::Deny(format!(
                    "这一段（{}）不发给 {} 的 {} 能力：{} 这一回合不能用",
                    seg.id, req.role, ability, req.tool
                ));
            }
        }
        Verdict::Allow
    }
}

/// 目的：装配时注入的默认策略。
pub fn default_policy() -> Arc<dyn Policy> {
    Arc::new(LoopPolicy)
}

/// 目的：一个能力组覆盖哪些工具 id——来源（发放）与策略（抑制）共用这一份映射，避免两处漂。
/// 参数：ability = 能力组 id（`fs-read` / `fs-write` / `exec` / `shared-write`）；不认识的组给空。
pub fn ids_of(ability: &str) -> &'static [&'static str] {
    use crate::units::business::tools::api as t;
    match ability {
        FS_READ => &[t::READ, t::LIST, t::GREP],
        FS_WRITE => &[t::WRITE, t::EDIT, t::PATCH, t::DELETE],
        EXEC => &[t::SHELL],
        SHARED_WRITE => crate::units::business::workspace::api::WORK_TOOLS,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：讨论段抑制写面与共享区写，读面与执行面不受影响。
    #[test]
    fn a_segment_suppresses_the_abilities_it_declares() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let p = LoopPolicy;
        let deny = |tool: &str| {
            matches!(
                p.decide(&ToolRequest {
                    tool,
                    sources: &sources,
                    loop_id: "discuss",
                    role: "discussant"
                }),
                Verdict::Deny(_)
            )
        };
        assert!(deny(crate::units::business::tools::api::WRITE));
        assert!(deny(crate::units::business::workspace::api::WORK_TOOLS[1]));
        assert!(!deny(crate::units::business::tools::api::READ));
        assert!(!deny(crate::units::business::tools::api::SHELL));
        assert!(!matches!(
            p.decide(&ToolRequest {
                tool: crate::units::business::tools::api::WRITE,
                sources: &sources,
                loop_id: "execute",
                role: "executor"
            }),
            Verdict::Deny(_)
        ));
    }
}
