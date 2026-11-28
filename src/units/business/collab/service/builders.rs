//! 目的：协作流程的注册项——`discuss` / `execute` 两个实现名交给注册表，工厂与装配点不再有分支。
//! 管：`PhaseBuilder`（名字 + 阶段 → `CollabSession`）、`flow_builders`（构造注册项）、`FLOW_BUILDERS`（名字表，装配期校验用）。
//! 不管：两段各自的推进（`pump.rs` 等）；注册表的查表与校验（在 `turn_state` 与组合根）。
//! 联动：`systools/loops.yaml` 的 `builder` 由这里落地；组合根在 `runtime` 的 `Core::new` 登记这两个实现。

use std::sync::Arc;

use crate::kernel::ports::Log;
use crate::units::business::catalog::modules::api::Modules;
use crate::units::business::llm::api::Llm;
use crate::units::business::prompt::api::Prompt;
use crate::units::business::tools::api::{ToolExec, Tools};
use crate::units::business::turn_state::api::{BuildRequest, Flow, FlowBuilder};

use super::collab::{CollabSession, Phase};

/// 目的：讨论段的实现名（与 `systools/loops.yaml` 的 `builder` 逐字相同）。
pub const DISCUSS_BUILDER: &str = "discuss";
/// 目的：执行段的实现名（与 `systools/loops.yaml` 的 `builder` 逐字相同）。
pub const EXECUTE_BUILDER: &str = "execute";
/// 目的：本单元注册的流程实现名：装配期拿它校验段声明，不许出现查不到实现的 `builder`。
pub const FLOW_BUILDERS: &[&str] = &[DISCUSS_BUILDER, EXECUTE_BUILDER];

/// 目的：两段共用的入站面（会话本体要的那批句柄；注册项按值持有，克隆即分享）。
#[derive(Clone)]
struct Shares {
    llm: Arc<dyn Llm + Send + Sync>,
    modules: Arc<dyn Modules + Send + Sync>,
    prompts: Arc<dyn Prompt>,
    systools: Arc<dyn Tools>,
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
}

/// 目的：协作段的注册项：一个实现名对应同一条 `CollabSession` 的某一个阶段。
struct PhaseBuilder {
    name: &'static str,
    phase: Phase,
    shares: Shares,
}

impl PhaseBuilder {
    /// 目的：装一个注册项（名字取 `FLOW_BUILDERS` 里的常量，阶段决定这一段跑什么）。
    fn new(name: &'static str, phase: Phase, shares: Shares) -> PhaseBuilder {
        PhaseBuilder {
            name,
            phase,
            shares,
        }
    }
}

impl FlowBuilder for PhaseBuilder {
    fn name(&self) -> &str {
        self.name
    }

    fn build(&self, req: &BuildRequest<'_>) -> Result<Box<dyn Flow>, String> {
        let setup = req.setup;
        // 有重建现场就按转录恢复本段，没有就新建（两段共用同一份落盘事实）。
        let session = match req.restore {
            Some((meta, events)) => CollabSession::restore(
                Arc::clone(&self.shares.llm),
                Arc::clone(&self.shares.modules),
                setup.settings.clone(),
                Arc::clone(&self.shares.prompts),
                Arc::clone(&self.shares.systools),
                Arc::clone(&self.shares.tools),
                Arc::clone(&self.shares.log),
                meta,
                events,
                setup.sandboxes.clone(),
                self.phase,
                Arc::clone(&setup.ask),
                req.identity.clone(),
                req.min_members,
            )?,
            None => {
                let channel = setup
                    .core_channel
                    .clone()
                    .ok_or_else(|| format!("流程 {} 新建时缺少核心模型通道", req.seg))?;
                CollabSession::start(
                    Arc::clone(&self.shares.llm),
                    Arc::clone(&self.shares.modules),
                    setup.settings.clone(),
                    channel,
                    Arc::clone(&self.shares.prompts),
                    Arc::clone(&self.shares.systools),
                    Arc::clone(&self.shares.tools),
                    Arc::clone(&self.shares.log),
                    setup.roster.clone(),
                    setup.delegated,
                    setup.sandboxes.clone(),
                    self.phase,
                    Arc::clone(&setup.ask),
                    req.identity.clone(),
                    req.min_members,
                )?
            }
        };
        Ok(Box::new(session))
    }
}

/// 目的：构造本单元的注册项（组合根登记它们，注册表按名字查）。
/// 参数：两段要的入站面（会话本体那批句柄，全部共享给注册项）。
/// 约束：这里是实现名的第二处：新增一种协作流程要加常量与注册项各一行，本单元之外没有分派点。
#[allow(clippy::too_many_arguments)]
pub fn flow_builders(
    llm: Arc<dyn Llm + Send + Sync>,
    modules: Arc<dyn Modules + Send + Sync>,
    prompts: Arc<dyn Prompt>,
    systools: Arc<dyn Tools>,
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
) -> Vec<Arc<dyn FlowBuilder>> {
    let shares = Shares {
        llm,
        modules,
        prompts,
        systools,
        tools,
        log,
    };
    let list: Vec<Arc<dyn FlowBuilder>> = vec![
        Arc::new(PhaseBuilder::new(
            DISCUSS_BUILDER,
            Phase::Discuss,
            shares.clone(),
        )),
        Arc::new(PhaseBuilder::new(EXECUTE_BUILDER, Phase::Execute, shares)),
    ];
    // 名字表与注册项必须同源：装配期校验按名字表判，漏一处就是「声明写得出、跑不起来」。
    debug_assert_eq!(
        list.iter().map(|b| b.name()).collect::<Vec<_>>(),
        FLOW_BUILDERS.to_vec(),
        "协作注册项与 FLOW_BUILDERS 不一致"
    );
    list
}
