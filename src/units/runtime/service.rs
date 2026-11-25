//! 目的：service 层：核心本体与手柄、长步骤编排、动作分发、代理入站桥；子模块共用的 import 也在这里。
//! 管：子模块声明与共用面（下层能力的 api 类型、压缩工厂、动作词汇），每个子模块只写自己那段编排。
//! 不管：任何用例的身体（在下层能力）；装配清单（组合根 main.rs）；呈现层的形状（api 与 presentation）。
//! 联动：api 经 Handle 把命令交给这里；子模块一律 `use super::*` 取共用面。

use crate::kernel::api::jobs::JobRegistry;
use crate::units::catalog::modules::api::{Module, Modules, Roster};
use crate::units::catalog::packages::api::Packages;
use crate::units::catalog::registry::api::Registry;
use crate::units::catalog::registry::api::{
    AgentView, AppSettings, ModelView, ProviderView, RegistryOps, StorageSettings,
};
use crate::units::conversation::proxy::api as d;
use crate::units::conversation::session::api::subtree_of;
use crate::units::conversation::session::api::{
    Answered, History, HistoryView, RunState, SessionMeta,
};
use crate::units::llm::api::Channel;
#[cfg(test)]
use crate::units::llm::api::Msg;
use crate::units::llm::api::{Llm, ProbeOutcome, ToolMode};
use crate::units::maintenance::builds::api::Builds;
use crate::units::prompt::api::Prompt;
use crate::units::provision::api::Provision;
use crate::units::pure::exec_model::api::Tier;
use crate::units::runtime::api::{
    Acted, ActionCall, ActionOps, ActionParamView, ActionView, Advance, Caller, CollabWork, Events,
    LogOps, Output, RebuildDeps, RewindOutcome, RewindTarget, RuntimeOps,
};
use crate::units::tools::api::{ToolExec, ToolSchema, Tools, arg_fault_text};
use crate::units::transcript::api::{DecisionQueue, SessionEvent};
use crate::units::turn::turn::api::{SPEAKER, TurnOps};
use crate::units::vm::api::Vm;
use crate::units::work::api::WorkOps;
use crate::units::work::api::{
    AgentSuggestion, EditGuest, FilesView, GuestBoot, PreparedActivation, PreparedEdit,
    PreparedWork, RuntimeReport, SessionConfig, SessionEdit, SessionTypeView, SessionView,
    TierChoices, Work, WorkOpened, WorkSpec,
};
use crate::units::workspace::api::{MAX_ASSET_BYTES, Workspace};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// 目的：代理入站桥（`bridge::ProxyBridge`）：注入成员侧执行者时造它。
pub(crate) use self::bridge::ProxyBridge;
/// 目的：压缩通道工厂的选择（`compact::CompactChoice`）：核心按全局设置对账它。
use self::compact::{CompactChoice, CompactRuntime};
/// 目的：核心本体（`core::Core`）：状态只在核心线程上被碰，测试装配点用它。
pub(crate) use self::core::Core;
/// 目的：会话本体（`turn` 的 `Turn`）：代理控制要问运行态。
pub(crate) use self::core::Turn;
/// 目的：核心手柄（`handle::Handle`）：`api` 与装配点只经它发命令。
pub(crate) use self::handle::Handle;

/// 目的：动作：目录、授权、分发与执行。
pub(crate) mod action;
/// 目的：启用的长步骤编排：补编模块、拉 guest、提交激活。
mod activation;
/// 目的：代理入站桥：`proxy::api::ProxyHost` 的宿主实现。
pub(crate) mod bridge;
/// 目的：压缩通道工厂：全局设置指向的压缩模型在这里解析。
pub(crate) mod compact;
/// 目的：入站契约的实现：每个方法只有一行 call 或一次转发。
mod contracts;
/// 目的：核心本体：状态容器、访问器与薄转发。
pub(crate) mod core;
/// 目的：guest 诊断的编排。
mod env;
/// 目的：核心手柄：命令队列、事件台发布、提示与长步骤入口。
pub(crate) mod handle;
/// 目的：回档的编排：停稳子树、算回档、推事件台。
mod rewind;
