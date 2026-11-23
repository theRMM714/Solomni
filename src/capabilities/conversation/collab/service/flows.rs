//! 目的：协作流程的工厂——把会话类型表里的 id 映射到讨论段 / 执行段，并按现场新建或恢复。
//! 管：`CollabFlows`（id → `CollabSession` 的某一段）；是组合根注入给 turn / work 的唯一入口。
//! 不管：两段各自的推进（`pump.rs` 等）；装配点的分支（turn 的重建与 work 的创建只认 `Flows`）。
//! 联动：`systools/profiles.yaml` 的流程序列由它落地；新增协作流程在这里加一条映射。

use std::sync::Arc;

use crate::capabilities::catalog::modules::api::Modules;
use crate::capabilities::conversation::session::api::{Flow, FlowSetup, Flows, Profiles};
use crate::capabilities::llm::api::Llm;
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::vm::api::Vm;
use crate::kernel::ports::Log;

use super::collab::{CollabSession, Phase};

/// 目的：协作流程工厂：持有装配两段要的能力面，按 id 造讨论段或执行段。
pub struct CollabFlows {
    llm: Arc<dyn Llm + Send + Sync>,
    vm: Arc<dyn Vm + Send + Sync>,
    modules: Arc<dyn Modules + Send + Sync>,
    prompts: Arc<dyn Prompt>,
    systools: Arc<dyn Tools>,
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    book: Profiles,
}

impl CollabFlows {
    /// 目的：组合根注入：能力面全是共享句柄（Arc 克隆，不新建实现），会话类型表是装好的那一份。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        llm: Arc<dyn Llm + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        prompts: Arc<dyn Prompt>,
        systools: Arc<dyn Tools>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        book: Profiles,
    ) -> CollabFlows {
        CollabFlows {
            llm,
            vm,
            modules,
            prompts,
            systools,
            tools,
            log,
            book,
        }
    }
}

impl Flows for CollabFlows {
    fn book(&self) -> &Profiles {
        &self.book
    }

    fn build(&self, id: &str, setup: &FlowSetup<'_>) -> Result<Box<dyn Flow>, String> {
        let phase = match id {
            "discuss" => Phase::Discuss,
            "execute" => Phase::Execute,
            other => return Err(format!("流程没注册：{}（systools/profiles.yaml）", other)),
        };
        // 两段共用同一份落盘事实：有重建现场就按转录恢复，没有就新建。
        let session = match setup.restore {
            Some((meta, events)) => CollabSession::restore(
                Arc::clone(&self.llm),
                Arc::clone(&self.vm),
                Arc::clone(&self.modules),
                setup.settings.clone(),
                Arc::clone(&self.prompts),
                Arc::clone(&self.systools),
                Arc::clone(&self.tools),
                Arc::clone(&self.log),
                meta,
                events,
                setup.sandboxes.clone(),
                phase,
                Arc::clone(&setup.desk),
            )?,
            None => {
                let channel = setup
                    .core_channel
                    .clone()
                    .ok_or_else(|| format!("流程 {} 新建时缺少核心模型通道", id))?;
                CollabSession::start(
                    Arc::clone(&self.llm),
                    Arc::clone(&self.vm),
                    Arc::clone(&self.modules),
                    setup.settings.clone(),
                    channel,
                    Arc::clone(&self.prompts),
                    Arc::clone(&self.systools),
                    Arc::clone(&self.tools),
                    Arc::clone(&self.log),
                    setup.roster.clone(),
                    setup.delegated,
                    setup.sandboxes.clone(),
                    phase,
                    Arc::clone(&setup.desk),
                )?
            }
        };
        Ok(Box::new(session))
    }
}
