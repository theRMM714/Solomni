//! 目的：工作台策略的契约——一席的工具、执行者、写入门禁与回合钩子由策略给出，会话只持策略引用。
//! 管：`BenchPolicy`（一个策略怎么开一席）、`BenchPolicies`（有哪些策略）、`BenchPlan`（开出来的东西）、
//!   `BenchDriver`（版本化那类策略在回合边界要做的四件事）。
//! 不管：实现（`workspace` 的 versioned 与 plain）；组合根接哪几个；布局怎么落盘。
//! 联动：`agentloop` 装配期调 `plan`、执行期调驱动；`turn` 注入策略表；视图按 `capabilities` 渲染。

use std::path::PathBuf;
use std::sync::Arc;

use crate::kernel::api::types::ToolOutcome;
use crate::kernel::ports::{Log, ToolHandler};

use crate::units::business::workbench::domain::bench::Bench;
use crate::units::business::workbench::domain::gate::WriteGate;
use crate::units::business::workbench::domain::observed::ObservationFact;
use crate::units::business::workbench::domain::readfact::ReadQuery;

/// 目的：两个内置策略的 id（会话声明里存的就是它们）。
pub const VERSIONED_POLICY: &str = "versioned";
/// 目的：不版本化的策略 id。
pub const PLAIN_POLICY: &str = "plain";

/// 目的：版本化能力的名字——视图按它决定要不要渲染回档 / 留档类按键。
pub const VERSIONS_CAP: &str = "versions";

/// 目的：共享区（主副本）那一组动作的 id——版本化策略按它整组发放，被抑制时按它整组拦。
/// 约束：需要版本库才成立的动作，因此形状与拦截口径都留在契约单元，`versions` 只重导出。
pub const SHARED_TOOLS: &[&str] = &[
    "work_pull",
    "work_commit",
    "work_push",
    "work_status",
    "work_versions",
    "work_diff",
    "work_reset",
    "work_resolve",
    "work_lost",
    "work_sync",
    "work_ignore",
];

/// 目的：开一席工作面要的事实：这一席的工作面、它的能力组与策略参数。
pub struct PlanRequest<'a> {
    /// 目的：这一席的工作面（寻址与权限面已落值）。
    /// 约束：None = 只要名单（工具面诊断用）：策略交工具 id，不造执行者与回合钩子。
    pub bench: Option<&'a Bench>,
    /// 目的：这个 agent 的能力组（策略按它决定发不发某类工具）。
    pub abilities: &'a [String],
    /// 目的：策略参数（会话声明里那一段不透明 JSON；策略自己解释）。
    /// 约束：端口契约的字段，今天的 versioned / plain 都不读它——留给要旋钮的策略。
    #[allow(dead_code)]
    pub params: &'a serde_json::Value,
}

/// 目的：一个策略为一席开出来的东西：工具 id、它们的执行者、回合钩子。
#[derive(Default)]
pub struct BenchPlan {
    /// 目的：这一席由策略发放的工具 id（版本化策略的 11 个 `work_*`；不版本化时为空）。
    pub tools: Vec<String>,
    /// 目的：这些 id 的执行者（`kernel::ports::ToolHandler`）。
    pub handlers: Vec<Arc<dyn ToolHandler>>,
    /// 目的：回合边界要做的版本化动作；不版本化 = None（引擎不认识版本库）。
    pub driver: Option<Arc<dyn BenchDriver>>,
}

/// 目的：造门禁要的现场：哪条工作线、哪棵树、当时的观察事实、拒绝时往哪记审计。
pub struct GateRequest<'a> {
    /// 目的：工作名（定位工作线）。
    pub work: &'a str,
    /// 目的：agent 名（定位它的工作线）。
    pub agent: &'a str,
    /// 目的：这些事实属于哪棵工作树（空 = 主副本）。
    pub worktree: &'a str,
    /// 目的：批前的观察快照。
    pub facts: &'a [ObservationFact],
    /// 目的：拒绝时记审计日志（只记路径与版本，不记内容）。
    pub log: Option<Arc<dyn Log>>,
}

/// 目的：一次工具调用之前的现场（版本化策略据此拦未读的已追踪路径）。
pub struct CallRequest<'a> {
    /// 目的：这一席的工作面。
    pub bench: &'a Bench,
    /// 目的：工具名。
    pub name: &'a str,
    /// 目的：调用参数原文。
    pub args_json: &'a str,
    /// 目的：批前的观察快照。
    pub facts: &'a [ObservationFact],
}

/// 目的：一次写工具落地之后的事实（版本化策略据此把改动记进存档线）。
pub struct WriteDone<'a> {
    /// 目的：这一席的工作面。
    pub bench: &'a Bench,
    /// 目的：这次调用真正写过的绝对路径。
    pub written: &'a [PathBuf],
    /// 目的：这次调用删掉的绝对路径。
    pub removed: &'a [PathBuf],
    /// 目的：这次调用所属的转录行锚。
    pub line: u64,
}

/// 目的：一批调用结束时的现场（版本化策略据此采纳 shell / 模块工具的改动）。
pub struct BatchDone<'a> {
    /// 目的：这一席的工作面。
    pub bench: &'a Bench,
    /// 目的：这一批结束时的观察快照。
    pub facts: &'a [ObservationFact],
    /// 目的：这一批最后一个工具调用所属的转录行锚。
    pub line: u64,
}

/// 目的：版本化工作台在回合边界要做的事——引擎只按这个契约调，不认识版本库。
/// 约束：默认全部不做（不版本化的策略不实现任何一条）；实现方不得改观察账本。
pub trait BenchDriver: Send + Sync {
    /// 目的：造这一次调用用的写入门禁；None = 这一席不拦写。
    fn gate(&self, _req: &GateRequest<'_>) -> Option<Arc<dyn WriteGate>> {
        None
    }
    /// 目的：调用之前的准入（`work_commit` 不能提交未读的已追踪路径）；Some = 拒绝回执。
    fn before(&self, _req: &CallRequest<'_>) -> Option<ToolOutcome> {
        None
    }
    /// 目的：写工具落地后记进存档线；Some = 挂到回执后的说明。
    fn after_write(&self, _req: &WriteDone<'_>) -> Option<String> {
        None
    }
    /// 目的：一批结束后的采纳（shell / 模块工具改的东西）；Some = 挂到最后一个不透明工具回执后的说明。
    fn after_batch(&self, _req: &BatchDone<'_>) -> Option<String> {
        None
    }
    /// 目的：判定一组读取是否已过期（上下文保真用）；None = 这一席不做过期判定。
    fn stale_reads(&self, _work: &str, _agent: &str, _queries: &[ReadQuery]) -> Option<Vec<bool>> {
        None
    }
}

/// 目的：一个工作台策略：id、它声明的能力位、以及怎么开一席。
pub trait BenchPolicy: Send + Sync {
    /// 目的：策略 id（会话声明里存的就是它）。
    fn id(&self) -> &'static str;
    /// 目的：这个策略提供的能力位（视图据此渲染回档 / 留档类按键）。
    fn capabilities(&self) -> &'static [&'static str];
    /// 目的：这个策略是不是把共享区当可写工作树（不版本化时是单树直写）。
    fn shared_writable(&self) -> bool;
    /// 目的：外部工作区首次启用时的初始导入（只有版本化策略有这一步；不版本化是空操作）。
    /// 错误：导入失败如实报给调用方（重建据此记一条告警，不拦会话）。
    fn import_root(&self, _work: &str, _at: i64) -> Result<(), String> {
        Ok(())
    }
    /// 目的：按这一席的事实开出工具、执行者与回合钩子。
    /// 错误：参数不合法或工作面缺事实时逐字报给调用方（装配期失败即这一席没有工具）。
    fn plan(&self, req: &PlanRequest<'_>) -> Result<BenchPlan, String>;
}

/// 目的：工作台策略表——组合根接哪几个由它决定；会话只按 id 取。
pub trait BenchPolicies: Send + Sync {
    /// 目的：按 id 取策略；没有 = None（调用方如实报「没有这个工作区策略」）。
    fn get(&self, id: &str) -> Option<Arc<dyn BenchPolicy>>;
}

/// 目的：一张按 id 取的策略表——组合根把哪几个策略接进来由它决定，会话只按 id 取。
/// 约束：首个即缺省；id 重复时先来的生效（装配期问题由调用方在 `main` 报）。
pub struct Policies {
    list: Vec<Arc<dyn BenchPolicy>>,
}

impl Policies {
    /// 目的：按顺序接一组策略（第一个是缺省）。
    pub fn new(list: Vec<Arc<dyn BenchPolicy>>) -> Self {
        Self { list }
    }
}

impl BenchPolicies for Policies {
    fn get(&self, id: &str) -> Option<Arc<dyn BenchPolicy>> {
        self.list.iter().find(|p| p.id() == id).cloned()
    }
}
