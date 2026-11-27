//! 目的：工具调用的授权端口——「这一刻准不准调」，与「在不在场」（来源各自交 id）分开。
//! 管：`Policy` 端口、默认实现 `LoopPolicy`（按流程段的抑制表判）与能力组到工具 id 的映射。
//! 不管：工具面怎么合成（`assemble`）、工具形状（`tools` 总表）、沙箱档位（`permission`）。
//! 联动：装配时按当前段拍平成值注入 `MemberTools`（`LoopPolicy::of`）；引擎每次调用前问它（`src/units/business/agentloop/engine/tools.rs`）。

use std::sync::Arc;

use crate::units::business::session_model::api::{EXEC, FS_READ, FS_WRITE, Loops, SHARED_WRITE};

/// 目的：一次调用的授权请求：调哪个工具、由哪些来源交出、在哪个流程段的哪个角色下。
pub struct ToolRequest<'a> {
    /// 目的：要调的工具 id（本回合工具面里的一个）。
    pub tool: &'a str,
    /// 目的：这一刻把该 id 交出来的来源 id（`agent` / `module` / `loop` / `workspace` / `profile`）。
    /// 约束：默认策略只按段判；卡片把它原样写进载荷，事后能看出是谁交出了这个工具。
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
    /// 目的：要问一次——先走会话的裁决通道拿一个答复，答了才执行。
    /// 约束：`why` 只说为什么问（进卡片正文）；卡片的文字与选项由 [`ask_card`] 统一生成。
    Ask {
        /// 目的：为什么这一次要点头（进卡片正文，模型与用户都看得懂）。
        why: String,
    },
    /// 目的：不准，理由进工具回执（模型读到原因自己换路）。
    Deny(String),
}

/// 目的：授权端口：这一次调用准不准。
/// 约束：只回答准不准；在场与否由来源回答（端口不合并两者的判据）。
pub trait Policy: Send + Sync {
    /// 目的：判这一次调用。
    fn decide(&self, req: &ToolRequest<'_>) -> Verdict;
}

/// 目的：默认策略：流程段声明抑制哪些能力组（`Loop.deny`）与哪些动作要点头（`Loop.ask`）。
/// 约束：装配期按当前段拍平成值（`of`）——运行期不再查段表；段表在数据里（`systools/loops.yaml`），代码里没有逐工具的名单。
pub struct LoopPolicy {
    id: String,
    deny: Vec<String>,
    ask: Vec<String>,
}

impl LoopPolicy {
    /// 目的：按一段的声明拍平出这一席的策略；段没在册就是「不抑制」。
    pub fn of(loops: &Loops, loop_id: &str) -> LoopPolicy {
        match loops.of(loop_id) {
            Some(l) => LoopPolicy {
                id: l.id.clone(),
                deny: l.deny.clone(),
                ask: l.ask.clone(),
            },
            None => LoopPolicy::none(),
        }
    }

    /// 目的：不抑制也不问（核实环境与测试替身用）。
    pub fn none() -> LoopPolicy {
        LoopPolicy {
            id: String::new(),
            deny: Vec::new(),
            ask: Vec::new(),
        }
    }
}

impl Policy for LoopPolicy {
    fn decide(&self, req: &ToolRequest<'_>) -> Verdict {
        if self.id.is_empty() {
            return Verdict::Allow;
        }
        verdict_of(&self.id, &self.deny, &self.ask, req)
    }
}

/// 目的：按段声明判一次调用：先看抑制（能力组），再看要不要问（工具 id），都没有就是准。
/// 约束：抑制先于问——一段里已经被抑制的动作不会再去问用户（问了也不能放行）。
/// 参数：`id` = 段 id（只用于文案）；`deny` = 抑制的能力组；`ask` = 要点头的工具 id；`req` = 这一次调用。
pub fn verdict_of(id: &str, deny: &[String], ask: &[String], req: &ToolRequest<'_>) -> Verdict {
    for ability in deny {
        if ids_of(ability).contains(&req.tool) {
            return Verdict::Deny(format!(
                "这一段（{}）不发给 {} 的 {} 能力：{} 这一回合不能用",
                id, req.role, ability, req.tool
            ));
        }
    }
    if ask.iter().any(|t| t == req.tool) {
        return Verdict::Ask {
            why: format!(
                "这一段（{}）里的 {} 要你点头才执行：{}",
                id, req.role, req.tool
            ),
        };
    }
    Verdict::Allow
}

/// 目的：这一问的两个选项 id——**行为契约**（回答按 id 分派，改文案不改行为）。
pub const OPT_ALLOW: &str = "allow";
/// 目的：拒绝选项 id（与 [`OPT_ALLOW`] 成对）。
pub const OPT_DENY: &str = "deny";

/// 目的：把「要问」变成一张卡：谁在问、问什么、几个选项都由这一处定，策略只给理由。
/// 参数：`agent` = 撞上这一问的 agent 实例名（信封第二段）；`req` = 这一次调用；`why` = 策略给的理由。
/// 约束：选项恒为「允许这一次 / 不允许」两条：没有置灰，也没有默认项（没人答就是没放行）。
pub fn ask_card(agent: &str, req: &ToolRequest<'_>, why: &str) -> crate::kernel::api::types::Ask {
    crate::kernel::api::types::Ask {
        role: "tools".to_string(),
        name: agent.to_string(),
        title: format!("这一段要你点头：{}", req.tool),
        body: why.to_string(),
        detail: format!("流程段 {} · 角色 {}", req.loop_id, req.role),
        options: vec![
            (OPT_ALLOW.to_string(), "允许这一次".to_string()),
            (OPT_DENY.to_string(), "不允许".to_string()),
        ],
        kind: "policy".to_string(),
        payload: serde_json::json!({
            "loop": req.loop_id,
            "role": req.role,
            "tool": req.tool,
            "sources": req.sources,
        }),
        notes: Vec::new(),
    }
}

/// 目的：装配时注入的默认策略（不抑制、不问）。
pub fn default_policy() -> Arc<dyn Policy> {
    Arc::new(LoopPolicy::none())
}

/// 目的：一个能力组覆盖哪些工具 id——来源（发放）与策略（抑制）共用这一份映射，避免两处漂。
/// 参数：ability = 能力组 id（`fs-read` / `fs-write` / `exec` / `shared-write`）；不认识的组给空。
pub fn ids_of(ability: &str) -> &'static [&'static str] {
    use crate::units::business::tools::api as t;
    match ability {
        FS_READ => &[t::READ, t::LIST, t::GREP],
        FS_WRITE => &[t::WRITE, t::EDIT, t::PATCH, t::DELETE],
        EXEC => &[t::SHELL],
        SHARED_WRITE => crate::units::business::versions::api::WORK_TOOLS,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(tool: &'a str, sources: &'a [String]) -> ToolRequest<'a> {
        ToolRequest {
            tool,
            sources,
            loop_id: "asking",
            role: "solo",
        }
    }

    /// 目的：段声明了要问，段里这个动作给「问」；没声明的动作照旧放行。
    #[test]
    fn a_declared_action_asks_before_it_runs() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let ask = [crate::units::business::tools::api::WRITE.to_string()];
        assert!(matches!(
            verdict_of(
                "asking",
                &[],
                &ask,
                &req(crate::units::business::tools::api::WRITE, &sources)
            ),
            Verdict::Ask { .. }
        ));
        assert!(matches!(
            verdict_of(
                "asking",
                &[],
                &ask,
                &req(crate::units::business::tools::api::READ, &sources)
            ),
            Verdict::Allow
        ));
    }

    /// 目的：抑制先于问：一段既抑制又声明要问的动作不会被问（问了也放行不了）。
    #[test]
    fn a_suppressed_action_is_never_asked_about() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let deny = [FS_WRITE.to_string()];
        let ask = [crate::units::business::tools::api::WRITE.to_string()];
        assert!(matches!(
            verdict_of(
                "both",
                &deny,
                &ask,
                &req(crate::units::business::tools::api::WRITE, &sources)
            ),
            Verdict::Deny(_)
        ));
    }

    /// 目的：卡片形状是行为契约：两个固定选项 id、信封两段、载荷带段与角色。
    #[test]
    fn the_card_says_who_asks_and_what_the_options_are() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let card = ask_card(
            "甲",
            &req(crate::units::business::tools::api::WRITE, &sources),
            "要你点头",
        );
        assert_eq!(card.role, "tools");
        assert_eq!(card.name, "甲");
        assert!(card.has_option(OPT_ALLOW) && card.has_option(OPT_DENY));
        assert_eq!(card.options.len(), 2);
        assert_eq!(card.kind, "policy");
        assert_eq!(
            card.payload["tool"],
            crate::units::business::tools::api::WRITE
        );
    }

    /// 目的：讨论段抑制写面与共享区写，读面与执行面不受影响。
    #[test]
    fn a_segment_suppresses_the_abilities_it_declares() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let p = LoopPolicy {
            id: "discuss".to_string(),
            deny: vec![FS_WRITE.to_string(), SHARED_WRITE.to_string()],
            ask: Vec::new(),
        };
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
        assert!(deny(crate::units::business::versions::api::WORK_TOOLS[1]));
        assert!(!deny(crate::units::business::tools::api::READ));
        assert!(!deny(crate::units::business::tools::api::SHELL));
        let open = LoopPolicy::none();
        assert!(!matches!(
            open.decide(&ToolRequest {
                tool: crate::units::business::tools::api::WRITE,
                sources: &sources,
                loop_id: "execute",
                role: "executor"
            }),
            Verdict::Deny(_)
        ));
    }
}
