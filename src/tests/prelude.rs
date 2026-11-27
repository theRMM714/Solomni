//! 目的：集中重导出业务测试的公共前置，保持 `use super::prelude::*`。
//! 管：公共前置的重导出清单。
//! 不管：任何逻辑或断言。
//! 联动：src/tests.rs、tests/doubles。

pub(crate) use super::doubles::*;

pub use crate::units::business::collab::service::discussion::{
    Discussion, MAX_ROUNDS, Member, TurnOut,
};

pub use crate::units::business::llm::api::Channel;

pub use crate::units::business::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};

pub use crate::units::business::llm::detail::fake_chat::FakeChat;

pub use crate::units::business::llm::ports::ChatGateway;

pub use crate::units::business::prompt::api::{Prompt, Segment};

pub use crate::units::business::prompt::domain::prompt::render;

pub use crate::units::business::catalog::registry::api::Settings;
pub use crate::units::business::session_model::api::{RunState, SessionMeta};
pub use crate::units::pure::registry_model::api::AgentMeta;
pub use crate::units::pure::registry_model::api::ModelDraft;
pub use crate::units::pure::registry_model::api::ModelEntry;
pub use crate::units::pure::registry_model::api::Provider;

// 工具面活对象只在 `agentloop` 内构造与持有（`api` 不转出）：测试直接拿它来断言围栏与续号。
pub use crate::units::business::agentloop::api::ModuleTools;
pub(crate) use crate::units::business::agentloop::domain::member_tools::MemberTools;

pub use crate::units::business::conversation::session::ports::HistoryStore;

pub use crate::kernel::ports::ProcessRunner;
pub use crate::units::business::tools::api::ToolExec;

pub use crate::units::business::catalog::modules::api::Module;

pub use crate::units::business::vm::api::{self as exec};
pub use crate::units::pure::exec_model::api::TargetSpec;

pub use crate::units::business::catalog::modules::ports::ModuleSource;
pub use crate::units::business::workspace::ports::Workdirs;

pub use crate::units::business::transcript::api::SessionEvent;
pub use crate::units::business::work::api::{ConfigAgent, SessionEdit, WorkSpec};

pub use crate::units::business::collab::api::{CollabEvent, Pending};

/// 目的：裁决卡的选项 id（契约）：测试按它作答，不按文案。
pub use crate::units::business::collab::api::{OPT_BEGIN, OPT_PLAN_SAY, OPT_SLATE_CONFIRM};

pub(crate) use crate::units::business::runtime::service::Core;

pub use crate::units::pure::exec_model::api::Side;

pub use std::collections::{BTreeMap, BTreeSet};

pub use std::path::PathBuf;

pub use std::sync::atomic::{AtomicUsize, Ordering};

pub use std::sync::{Arc, Mutex};

pub use std::time::Duration;
