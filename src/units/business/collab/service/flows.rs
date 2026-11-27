//! 目的：协作流程的工厂——把段声明里的实现名（`builder`）映射到讨论段 / 执行段，并按现场新建或恢复。
//! 管：`CollabFlows`（实现名 → `CollabSession` 的某一段）；是组合根注入给 turn / work 的唯一入口。
//! 不管：两段各自的推进（`pump.rs` 等）；装配点的分支（turn 的重建与 work 的创建只认 `Flows`）。
//! 联动：`systools/loops.yaml` 的 `builder` 由它落地；新增协作流程在这里加一条实现名映射。

use std::sync::Arc;

use crate::kernel::ports::Log;
use crate::units::business::catalog::modules::api::Modules;
use crate::units::business::llm::api::Llm;
use crate::units::business::prompt::api::Prompt;
use crate::units::business::session_model::api::SessionDecls;
use crate::units::business::tools::api::{ToolExec, Tools};
use crate::units::business::turn_state::api::{Flow, FlowSetup, Flows};
use crate::units::business::vm::api::Vm;

use super::collab::{CollabSession, Phase};

/// 目的：协作流程工厂：持有装配两段要的入站面，按 id 造讨论段或执行段。
pub struct CollabFlows {
    llm: Arc<dyn Llm + Send + Sync>,
    vm: Arc<dyn Vm + Send + Sync>,
    modules: Arc<dyn Modules + Send + Sync>,
    prompts: Arc<dyn Prompt>,
    systools: Arc<dyn Tools>,
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    decls: Arc<SessionDecls>,
}

impl CollabFlows {
    /// 目的：组合根注入：入站面全是共享句柄（Arc 克隆，不新建实现），会话类型表是装好的那一份。
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
        decls: Arc<SessionDecls>,
    ) -> CollabFlows {
        CollabFlows {
            llm,
            vm,
            modules,
            prompts,
            systools,
            tools,
            log,
            decls,
        }
    }
}

impl Flows for CollabFlows {
    fn decls(&self) -> &SessionDecls {
        &self.decls
    }

    fn build(&self, id: &str, setup: &FlowSetup<'_>) -> Result<Box<dyn Flow>, String> {
        // 段 id 与实现的对应关系只在数据里：这里只认段声明写的实现名。
        let loops = self.decls.loops();
        let seg = loops
            .of(id)
            .ok_or_else(|| format!("流程没在册：{}（systools/loops.yaml）", id))?;
        let phase = match seg.builder.as_deref() {
            Some("discuss") => Phase::Discuss,
            Some("execute") => Phase::Execute,
            Some(other) => {
                return Err(format!(
                    "协作工厂里没有这个实现名：{}（流程段 {}；systools/loops.yaml）",
                    other, id
                ));
            }
            None => {
                return Err(format!("流程段 {} 没声明实现名（systools/loops.yaml）", id));
            }
        };
        // 这一段的成员身份与最少成员数按段声明算一次，交进会话（会话不认识具体段 id）。
        let identity = loops
            .member_role(id)
            .map(|role| (id.to_string(), role.to_string()));
        let min_members = loops.min_members(id);
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
                Arc::clone(&setup.ask),
                identity.clone(),
                min_members,
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
                    Arc::clone(&setup.ask),
                    identity,
                    min_members,
                )?
            }
        };
        Ok(Box::new(session))
    }
}
