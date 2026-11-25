//! 目的：`registry_model` 的对外契约——agent 的存档形态、视图与名字规则。
//! 管：本单元对其它单元暴露的全部形状与函数。
//! 不管：实现（在 `domain/`）；名单落地（在 catalog）。
//! 联动：catalog、turn、runtime 与呈现层只经这里取 agent 形状（R15：会话名单的唯一形状）。

#[allow(unused_imports)]
pub use crate::units::pure::registry_model::domain::agents::{
    Agent, AgentDraft, AgentMeta, AgentView, Agents, RosterPick, agent_meta, check_reserved,
    listing, pick, validate_name, views,
};
