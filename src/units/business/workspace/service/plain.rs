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
