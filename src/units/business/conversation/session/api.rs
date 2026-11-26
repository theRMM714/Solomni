//! 目的：会话单元的入站面——其它单元与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：重导出落盘手柄、会话树派生与流程段自洽断言。
//! 不管：`domain` / `ports` 的内部实现；裁决队与流程面（在 `turn_state`）；落盘与队列两个面由 `turn_state` 定义；
//!   会话运行时（在 `agentloop`）；会话身份与声明（在 `session_model`）。
//! 联动：别的单元经 `turn_state::api::History` 造会话、追流水、读元信息、删会话；`turn` 从 `turn_state` 取流程面、
//!   从 `agentloop::api` 取整场装配（它构造工具面活对象，装配机必须与工具面同单元）；呈现层经 runtime 的队列代理调 `HistoryOps`。

// 流程段与角色声明在 `session_model`；这里只留自洽断言（它要同时读工具总表与工具面，读不了环 2 的纯值单元）。
#[allow(unused_imports)]
pub use crate::units::business::conversation::session::domain::loop_check::problems as loop_problems;
// 落盘手柄：工作线程按"一次模型调用"的粒度增量落盘。
#[allow(unused_imports)]
pub use crate::units::business::conversation::session::service::{
    Persister, persist_events, record_events,
};
// 会话树派生：父子关系、工作根、配置归属、树级装配 meta 与 guest 作用域（只读 `History`）。
#[allow(unused_imports)]
pub use crate::units::business::conversation::session::service::tree::{
    config_owner_of, flat_modules, guest_lease, guest_lease_for, guest_scope, subtree_of,
    tree_meta, tree_meta_with, tree_order, work_root, work_root_of,
};
