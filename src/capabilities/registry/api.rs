//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::registry::domain::agents::{
    listing, model_listing, pick, resolve_picks, unique_instance_name, validate_name, views, Agent,
    AgentView, Agents, RosterPick,
};
pub use crate::capabilities::registry::domain::providers::{
    AppSettings, ModelEntry, ModelView, Provider, ProviderView, Settings,
};
