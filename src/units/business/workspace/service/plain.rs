//! 目的：不版本化的工作区策略——所有 agent 直接写同一棵工作树，没有共享区分界、没有 work_* 动作。
//! 管：`PlainPolicy`（id / 能力位 / 共享区可写 / 不开任何工具与钩子）。
//! 不管：树怎么摆（`workspace` 的席位组装）；版本化的那套（`versions` 的 versioned 策略）。
//! 联动：`turn` 按 `shared_writable` 造席位工作面；`agentloop` 装配期拿到空计划。

use crate::units::business::workbench::api::{BenchPlan, BenchPolicy, PLAIN_POLICY, PlanRequest};

/// 目的：不版本化的工作区策略：单树直写，工具面与回合钩子都为空。
pub struct PlainPolicy;

impl BenchPolicy for PlainPolicy {
    fn id(&self) -> &'static str {
        PLAIN_POLICY
    }

    fn capabilities(&self) -> &'static [&'static str] {
        &[]
    }

    fn shared_writable(&self) -> bool {
        true
    }

    fn plan(&self, _req: &PlanRequest<'_>) -> Result<BenchPlan, String> {
        Ok(BenchPlan::default())
    }
}

/// 目的：造这个策略（组合根接策略表时用）。
pub fn plain_policy() -> std::sync::Arc<dyn BenchPolicy> {
    std::sync::Arc::new(PlainPolicy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::business::workbench::api::PlanRequest;

    /// 目的：不版本化只负责「这棵树可写」：不给共享区动作、不给回合钩子——引擎的版本库入口只有这两处，所以零调用。
    #[test]
    fn plain_hands_out_no_versioning_tools_and_no_hooks() {
        let policy = plain_policy();
        let abilities = vec!["shared-write".to_string()];
        let params = serde_json::Value::Null;
        let plan = policy
            .plan(&PlanRequest {
                bench: None,
                abilities: &abilities,
                params: &params,
            })
            .expect("不版本化给空计划");
        assert!(plan.tools.is_empty(), "没有共享区动作：{:?}", plan.tools);
        assert!(plan.handlers.is_empty());
        assert!(
            plan.driver.is_none(),
            "不版本化没有回合边界钩子：引擎只经驱动与处理器碰版本库"
        );
        assert!(policy.shared_writable(), "共享区就是工作区");
        assert!(
            policy.capabilities().is_empty(),
            "没有能力位：前端不渲染回档类按键"
        );
        assert_eq!(policy.id(), PLAIN_POLICY);
    }
}
