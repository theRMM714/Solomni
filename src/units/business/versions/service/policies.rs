//! 目的：版本化的工作区策略——共享区是主副本（只读），改动经工作副本与十一个 work_* 动作提交。
//! 管：`VersionedPolicy`（id / 能力位 / 不开就为空 / 按能力组发放共享区动作）与它开的驱动。
//! 不管：动作本身（`service/work_tools`）；门禁与回合钩子（`service/driver`）；树怎么摆（`workspace`）。
//! 联动：`turn` 按 `shared_writable`（false）造席位工作面；`agentloop` 装配期拿到工具 id、执行者与驱动。

use crate::units::business::versions::api::Versions;
use crate::units::business::versions::service::driver::versioned_driver;
use crate::units::business::versions::service::work_tools::work_handlers;
use crate::units::business::workbench::api::{
    BenchPlan, BenchPolicy, PlanRequest, SHARED_TOOLS, VERSIONED_POLICY, VERSIONS_CAP,
};
use std::sync::Arc;

/// 目的：共享区版本化的能力组名——只有带它的 agent 才会拿到十一个 work_*。
const SHARED_WRITE: &str = "shared-write";

/// 目的：版本化的工作区策略：共享区只读、工具面 = 共享区动作、回合边界由驱动接管。
pub struct VersionedPolicy {
    versions: Arc<dyn Versions + Send + Sync>,
}

impl BenchPolicy for VersionedPolicy {
    fn id(&self) -> &'static str {
        VERSIONED_POLICY
    }

    fn capabilities(&self) -> &'static [&'static str] {
        &[VERSIONS_CAP]
    }

    fn shared_writable(&self) -> bool {
        false
    }

    fn import_root(&self, work: &str, at: i64) -> Result<(), String> {
        self.versions.work_import_root(work, at).map(|_| ())
    }

    fn plan(&self, req: &PlanRequest<'_>) -> Result<BenchPlan, String> {
        if !req.abilities.iter().any(|a| a == SHARED_WRITE) {
            return Ok(BenchPlan::default());
        }
        let tools: Vec<String> = SHARED_TOOLS.iter().map(|t| t.to_string()).collect();
        let Some(bench) = req.bench else {
            return Ok(BenchPlan {
                tools,
                ..BenchPlan::default()
            });
        };
        let handlers = work_handlers(&self.versions, bench, &tools);
        Ok(BenchPlan {
            tools,
            handlers,
            driver: Some(versioned_driver(Arc::clone(&self.versions))),
        })
    }
}

/// 目的：造版本化策略（组合根接策略表时用）。
pub fn versioned_policy(versions: Arc<dyn Versions + Send + Sync>) -> Arc<dyn BenchPolicy> {
    Arc::new(VersionedPolicy { versions })
}
