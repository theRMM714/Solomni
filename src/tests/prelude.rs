//! 测试公共前置：从原 `tests/core.rs` 拆出来的业务测试一律 `use super::prelude::*;`。
//! 这里只做**重导出**（原 `tests/core.rs` 顶部的 imports 集中一处），不放逻辑。

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

pub use crate::capabilities::registry::api::{ModelEntry, Provider, Settings};

pub use crate::capabilities::session::api::Live;

pub use crate::capabilities::session::api::{AgentMeta, SessionMeta};

pub use crate::capabilities::session::api::MemberTools;
pub use crate::capabilities::session::domain::tools::ModuleTools;

pub use crate::capabilities::session::ports::HistoryStore;

pub use crate::capabilities::tools::api::{ToolExec, ToolOutcome};
pub use crate::capabilities::tools::ports::ToolRunner;

pub use crate::capabilities::workspace::api::Module;

pub use crate::capabilities::workspace::api::{self as exec, Diagnosis, ExecSpec};

pub use crate::capabilities::workspace::api::{Library, PackageManifest};

pub use crate::capabilities::workspace::ports::{ModuleSource, Workdirs};

pub use crate::capabilities::conductor::api::{
    AgentInstance, CollabStep, ConfigAgent, Pending, SessionEdit, SessionEvent, WorkMode, WorkSpec,
};

pub use crate::capabilities::conductor::service::Conductor;

pub use crate::kernel::api::Tier;

pub use std::collections::{BTreeMap, BTreeSet};

pub use std::path::PathBuf;

pub use std::sync::atomic::{AtomicUsize, Ordering};

pub use std::sync::{Arc, Mutex};

pub use std::time::Duration;
// ---------- 信封 ----------
