//! 目的：集中重导出业务测试的公共前置，保持 `use super::prelude::*`。
//! 管：公共前置的重导出清单。
//! 不管：任何逻辑或断言。
//! 联动：src/tests.rs、tests/doubles。

pub(crate) use super::doubles::*;

pub use crate::capabilities::conversation::collab::service::discussion::{
    Discussion, MAX_ROUNDS, Member, TurnOut,
};

pub use crate::capabilities::llm::api::Channel;

pub use crate::capabilities::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};

pub use crate::capabilities::llm::detail::fake_chat::FakeChat;

pub use crate::capabilities::llm::ports::ChatGateway;

pub use crate::capabilities::prompt::api::{Prompt, Segment};

pub use crate::capabilities::prompt::domain::prompt::render;

pub use crate::capabilities::catalog::registry::api::{ModelDraft, ModelEntry, Provider, Settings};

pub use crate::capabilities::catalog::registry::api::AgentMeta;
pub use crate::capabilities::conversation::session::api::{RunState, SessionMeta};

pub use crate::capabilities::conversation::session::domain::tools::MemberTools;
pub use crate::capabilities::conversation::session::domain::tools::ModuleTools;

pub use crate::capabilities::conversation::session::ports::HistoryStore;

pub use crate::capabilities::tools::api::ToolExec;
pub use crate::kernel::ports::ProcessRunner;

pub use crate::capabilities::catalog::modules::api::Module;

pub use crate::capabilities::vm::api::{self as exec};
pub use crate::kernel::api::types::ExecSpec;

pub use crate::capabilities::catalog::modules::ports::ModuleSource;
pub use crate::capabilities::workspace::ports::Workdirs;

pub use crate::capabilities::runtime::api::SessionEvent;
pub use crate::capabilities::work::api::{ConfigAgent, SessionEdit, WorkSpec};

pub use crate::capabilities::conversation::collab::api::{CollabEvent, Pending};

/// 目的：裁决卡的选项 id（契约）：测试按它作答，不按文案。
pub use crate::capabilities::conversation::collab::api::{
    OPT_BEGIN, OPT_PLAN_SAY, OPT_SLATE_CONFIRM,
};

pub(crate) use crate::capabilities::runtime::service::Core;

pub use crate::kernel::api::types::Tier;

pub use std::collections::{BTreeMap, BTreeSet};

pub use std::path::PathBuf;

pub use std::sync::atomic::{AtomicUsize, Ordering};

pub use std::sync::{Arc, Mutex};

pub use std::time::Duration;
