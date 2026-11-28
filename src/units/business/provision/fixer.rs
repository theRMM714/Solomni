//! 目的：把补齐引擎接成执行环境策略要的 `Fixer` 端口——策略只说"缺哪几条依赖"，这里负责列清单与起作业（进度仍由呈现层读 `/api/setup/status`）。
//! 管：`ProvisionFixer`（`Fixer` 的适配器）与 `fixer` 入口。
//! 不管：缺什么（策略的 `machine` 说）、下载与解包（`detail` 与 `service`）。
//! 联动：组合根把它注入 `vm` 策略；呈现层只经 `sandbox` 的注册表问"能不能补"。

use crate::units::business::provision::api::{DepStatus, Provision};
use crate::units::pure::exec_model::api::{FixStep, Fixer};
use std::sync::Arc;

/// 目的：补齐端口的实现——只做 id 到清单行的翻译与作业直通，不自己判断缺什么。
pub struct ProvisionFixer {
    provision: Arc<dyn Provision + Send + Sync>,
}

impl ProvisionFixer {
    /// 目的：按补齐用例面建适配器（同一份实例，状态与作业都读它）。
    pub fn new(provision: Arc<dyn Provision + Send + Sync>) -> ProvisionFixer {
        ProvisionFixer { provision }
    }
}

impl Fixer for ProvisionFixer {
    fn plan(&self, ids: &[String]) -> Result<Vec<FixStep>, String> {
        let status = self.provision.status()?;
        // 没点名 = 列**现在缺的**（已在场的不重下、不重解）；点名 = 逐条按清单翻（点错了要硬报）。
        if ids.is_empty() {
            return Ok(status.iter().filter(|d| !d.present).map(step_of).collect());
        }
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let dep = status
                .iter()
                .find(|d| &d.id == id)
                .ok_or_else(|| format!("依赖清单里没有这一条：{}", id))?;
            out.push(step_of(dep));
        }
        Ok(out)
    }

    fn start(&self, ids: Vec<String>) -> Result<(), String> {
        self.provision.start(ids)
    }
}

/// 目的：一条依赖的展示事实——确认清单照抄它，界面不按 id 编话。
fn step_of(dep: &DepStatus) -> FixStep {
    FixStep {
        id: dep.id.clone(),
        label: if dep.note.is_empty() {
            dep.id.clone()
        } else {
            dep.note.clone()
        },
        url: dep.url.clone(),
        size_mb: dep.size_mb,
        note: format!("装到 {}（来源 {}）", dep.install, dep.source),
    }
}
