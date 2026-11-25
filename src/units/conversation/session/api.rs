//! 目的：会话能力的入站面——其它能力与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：重导出裁决队列、流程面、会话装配（参数对象与整场入口）、核心核实环境、落盘手柄与席位自洽断言；定义 `History` / `HistoryOps` 两个面。
//! 不管：`domain` / `ports` 的内部实现；`HistoryOps` 的队列代理实现（在 `runtime`）；会话运行时（在 `agentloop`）。
//! 联动：呈现层经 runtime 的队列代理调 `HistoryOps`；别的能力经 `History` 造会话、追流水、读元信息、删会话；`turn` 从这里取流程面与会话装配。

use crate::units::session_model::api::{HistoryView, SessionMeta};

pub use crate::units::conversation::session::domain::decisions::{
    Answered, Decisions, GateTicket, Gates, SlotWake, decisions, issued_in,
};
// 流程面：驱动只认这几个名字（推一步 / 让出一回合 / 交回后的处置 / 可派的活）。
pub use crate::units::conversation::session::domain::flow::{
    Flow, Handback, MemberAsk, Settle, WorkPlan,
};
// 席位与机制声明在 `session_model`；这里只留自洽断言（它要同时读工具总表与工具面，读不了环 2 的纯值单元）。
#[allow(unused_imports)]
pub use crate::units::conversation::session::domain::seat_check::problems as mechanism_problems;
// 流程工厂面：组合根把实现注入 turn / work，装配点只按 id 取段。
pub use crate::units::conversation::session::domain::flows::{FlowSetup, Flows, sequence_of};
// 会话装配：参数对象与三个整场入口（加上只取工具面值的 `member_face`）。
// `MemberTools`（工具面的活对象）**不经这里转出**：跨能力只传值结构（R16）。
#[cfg(test)]
pub use crate::units::conversation::session::service::assemble::member_face;
pub use crate::units::conversation::session::service::assemble::{
    MemberEnvParts, MemberSpec, ProxySpec, RebuildSpec, SessionPorts, SingleSpec, proxy_session,
    rebuild_single, single_session,
};
// 核心核实环境：`collab` 的各关卡按席位装配只读核实工具，装配逻辑留在本能力。
pub use crate::units::conversation::session::service::verify::{VerifySpec, verify_tools};
// 落盘手柄：工作线程按"一次模型调用"的粒度增量落盘。
#[allow(unused_imports)]
pub use crate::units::conversation::session::service::{Persister, persist_events, record_events};
// 会话树派生：父子关系、工作根、配置归属、树级装配 meta 与 guest 作用域（只读 `History`）。
#[allow(unused_imports)]
pub use crate::units::conversation::session::service::tree::{
    config_owner_of, flat_modules, guest_lease, guest_lease_for, guest_scope, subtree_of,
    tree_meta, tree_meta_with, tree_order, work_root, work_root_of,
};

/// 目的：落盘会话的**队列面**：呈现层经 runtime 的队列代理调它（列表 / 打开 / 删除）。
///   "在世会话 × 历史的并集"（`SessionView`）**不在本面里**：那要同时认识会话中心与历史，
///   归会话中心（`runtime::api::WorkOps::session_views`）。
pub trait HistoryOps: Send + Sync {
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}

/// 目的：会话的**直连面**（`service.rs` 实现）：别的能力要造会话、追流水、读元信息、删会话，走这里；
///   出站端口 `HistoryStore`（目录布局与 append-only 文件格式）**只由它持有**（R12）。
///   它与 `HistoryOps` 的分工是**接收者不同**，不是重复：`HistoryOps` 由队列代理（`Handle`）实现、
///   面向呈现层；`History` 由能力自己实现、面向别的能力。这里的四个写操作（create / save_meta /
///   append / delete）**不开放给呈现层**——呈现层要写就经协调业务的用例。
///   这一面刻意与端口**一一对应**：会话落盘没有别的不变式可编排（追加原子性、元信息唯一真相
///   已在 `session_model` 的 history 与 store 契约里），它的价值是**唯一持有者**（R12），不是新增逻辑。
pub trait History: Send + Sync {
    /// 建一个会话目录（meta + 空流水）。
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份的**唯一真相**在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 追加若干事件（留档只走这条；删除 / 恢复走 `replace`）。
    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    /// 整体重写流水（删除 / 恢复会真的截断）。
    fn replace(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String>;
    /// 列出全部落盘会话（列表页按它渲染）。
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    /// **只读元信息**（不回放流水）：运行态（暂停 / 关闭）判定走它，不为此读整份转录。
    fn meta(&self, name: &str) -> Result<SessionMeta, String>;
    /// 打开一个会话：元信息 + 事件流水（调用方按它回放状态）。
    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    /// 删除一个会话目录；false = 本来就不存在。
    fn delete(&self, name: &str) -> Result<bool, String>;
}
