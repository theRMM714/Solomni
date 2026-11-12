//! 目的：集中重导出业务测试的公共前置，保持 `use super::prelude::*`。
//! 管：公共前置的重导出清单。
//! 不管：任何逻辑或断言。
//! 联动：src/tests/mod.rs、tests/doubles。

pub(crate) use super::doubles::*;

pub use crate::capabilities::collab::service::discussion::{
    Discussion, Member, TurnOut, MAX_ROUNDS,
};

pub use crate::capabilities::llm::api::Channel;

pub use crate::capabilities::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};

pub use crate::capabilities::llm::detail::fake_chat::FakeChat;

pub use crate::capabilities::llm::ports::ChatGateway;

pub use crate::capabilities::prompt::api::{Prompt, Segment};

pub use crate::capabilities::prompt::domain::prompt::render;

pub use crate::capabilities::registry::api::{ModelDraft, ModelEntry, Provider, Settings};

pub use crate::capabilities::session::api::Live;

pub use crate::capabilities::session::api::{AgentMeta, RunState, SessionMeta};

pub use crate::capabilities::session::api::MemberTools;
pub use crate::capabilities::session::domain::tools::ModuleTools;

pub use crate::capabilities::session::ports::HistoryStore;

pub use crate::capabilities::tools::api::{ToolExec, ToolOutcome};
pub use crate::kernel::ports::ProcessRunner;

pub use crate::capabilities::modules::api::Module;

pub use crate::capabilities::vm::api::{self as exec};
pub use crate::capabilities::vm::domain::exec::EntryProblem as Diagnosis;
pub use crate::kernel::api::ExecSpec;

pub use crate::capabilities::modules::ports::ModuleSource;
pub use crate::capabilities::workspace::ports::Workdirs;

pub use crate::capabilities::conductor::api::{
    AgentInstance, ConfigAgent, DecisionCard, SessionEdit, SessionEvent, WorkMode, WorkSpec,
};

pub use crate::capabilities::session::api::Pending;

/// 目的：裁决卡的选项 id（契约）：测试按它作答，不按文案。
pub use crate::capabilities::session::api::{
    OPT_ASK_REPLY, OPT_BEGIN, OPT_BEGIN_ALLOW, OPT_NODE_REWORK, OPT_NODE_SAY, OPT_PLAN_SAY,
    OPT_PLAN_START, OPT_SLATE_CANCEL, OPT_SLATE_CONFIRM,
};

pub use crate::capabilities::conductor::service::Conductor;

pub use crate::kernel::api::Tier;

pub use std::collections::{BTreeMap, BTreeSet};

pub use std::path::PathBuf;

pub use std::sync::atomic::{AtomicUsize, Ordering};

pub use std::sync::{Arc, Mutex};

pub use std::time::Duration;
