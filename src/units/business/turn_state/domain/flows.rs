//! 目的：流程工厂面——装配期按会话类型表里的流程 id 造出一段流程（新增流程只在工厂里加一条映射）。
//! 管：`Flows`（id → 造一段）与 `FlowSetup`（造一段要的现场）；`sequence_of` 把若干段串成一条。
//! 不管：任何一段流程的内部；会话形状的声明（在 `systools/loops.yaml` 与 `systools/profiles.yaml`）；回合驱动（在 `turn`）。
//! 联动：组合根把实现注入 `turn` 与 `work`；装配点只按 id 取段，不认识任何具体流程的类型。

use std::sync::Arc;

use crate::units::business::catalog::registry::api::Settings;
use crate::units::business::llm::api::Channel;
use crate::units::business::session_model::api::{AskDesk, SessionDecls, SessionMeta};
use crate::units::business::turn_state::domain::flow::Flow;
use crate::units::business::turn_state::domain::sequence::Sequence;
use crate::units::business::workbench::api::Benches;
use crate::units::pure::registry_model::api::AgentMeta;

/// 目的：造一段流程要的现场（每一段按 id 自己取用，本段不用的部分不消费）。
pub struct FlowSetup<'a> {
    /// 目的：登记处快照（模型解析、工具形态与全局设置）。
    pub settings: Settings,
    /// 目的：核心模型通道：新建时要；从转录重建时按 `settings` 自己解出来，可不给。
    pub core_channel: Option<Channel>,
    /// 目的：本次工作的成员名单（代拟确认前为空）。
    pub roster: Vec<AgentMeta>,
    /// 目的：是否代拟（显式传入，不从名单是否为空推断）。
    pub delegated: bool,
    /// 目的：本工作的沙箱清单（流程按 agent 名取）。
    pub sandboxes: Benches,
    /// 目的：本会话的裁决通道**发起面**（各段共用同一条队的挂起口）。
    pub ask: Arc<dyn AskDesk>,
    /// 目的：重建现场：`Some` 就从转录恢复那一段，`None` 就是新建。
    pub restore: Option<(&'a SessionMeta, &'a [serde_json::Value])>,
}

/// 目的：流程工厂：会话形状的声明 + 段 id → 造一段（实现在流程自己的单元里，由组合根注入）。
pub trait Flows: Send + Sync {
    /// 目的：会话形状的声明表——装配按它取某个类型的流程序列，工厂按它查段声明的实现名。
    fn decls(&self) -> &SessionDecls;
    /// 目的：按段 id 造一段：新建还是按转录恢复，由 `setup.restore` 决定。
    /// 约束：段 id 与实现名的对应关系只在数据里（`systools/loops.yaml` 的 `builder`），本端口不出现具体段 id 的分支。
    /// 错误：段没在册或没声明实现名、现场缺必需项（例如新建却没有核心通道）时如实报错。
    fn build(&self, id: &str, setup: &FlowSetup<'_>) -> Result<Box<dyn Flow>, String>;
}

/// 目的：按会话类型声明的顺序造出整条流水线。
/// 参数：`ids` = 该类型的流程序列（`order` 即执行顺序）；`setup` = 共用现场；`flows` = 工厂。
/// 错误：任何一段造不出来即整体失败（装配期失败就失败，不半装配）。
pub fn sequence_of(
    ids: &[String],
    setup: &FlowSetup<'_>,
    flows: &dyn Flows,
) -> Result<Sequence, String> {
    let mut steps: Vec<Box<dyn Flow>> = Vec::with_capacity(ids.len());
    for id in ids {
        steps.push(flows.build(id, setup)?);
    }
    Ok(Sequence::new(steps))
}
