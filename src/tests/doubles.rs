//! 目的：测试替身与测试组合根（doubles）——内存适配器、共享夹具与装配辅助。
//! 管：各出站端口的内存实现、测试用入站面工厂、核心装配变体；本模块对“只有用例才用到”的项放行 dead_code。
//! 不管：用例本身（各业务测试文件）、真实适配器（`src/units/*/detail/`）。
//! 联动：语义规范见 docs/testing/doubles.md；端口契约测试与它同处一层（`src/tests/`）。

#![allow(dead_code)]

use super::builders::SilentRunner;
use crate::units::pure::registry_model::api::AgentMeta;
use crate::units::transcript::api::SessionEvent;
use crate::units::work::api::WorkSpec;

use crate::kernel::ports::ProcessRunner;
use crate::units::agentloop::api::TurnIo;
use crate::units::catalog::modules::api::{Module, ModuleManifest};
use crate::units::catalog::modules::ports::ModuleSource;
use crate::units::catalog::packages::domain::manifest::{Library, PackageManifest};
use crate::units::catalog::packages::ports::PackageSource;
use crate::units::catalog::registry::api::Settings;
use crate::units::catalog::registry::ports::SettingsStore;
use crate::units::conversation::session::ports::HistoryStore;
use crate::units::llm::api::Channel;
use crate::units::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};
use crate::units::llm::detail::fake_chat::FakeChat;
use crate::units::llm::ports::{ChatGateway, ModelCatalog};
use crate::units::prompt::api::Prompt;
use crate::units::prompt::domain::prompt::Prompts;
use crate::units::prompt::ports::PromptSource;
use crate::units::pure::exec_model::api::Tier;
use crate::units::pure::registry_model::api::ModelEntry;
use crate::units::pure::registry_model::api::Provider;
use crate::units::runtime::service::Core;
use crate::units::session_model::api::{HistoryView, SessionMeta};
use crate::units::tools::ports::SystoolsSource;
use crate::units::tools::ports::{FileRead, SysIo};
use crate::units::workspace::domain::asset::{AssetScanner, FileVersion};
use crate::units::workspace::ports::{WorkStore, Workdirs};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod assembly;
mod chat;
mod fixtures;
mod history;
mod host;
mod io;
mod probe;
mod proxy;
mod registry;
mod residents;
mod standin;
mod vm;
mod workspace;

pub(crate) use assembly::*;
pub(crate) use chat::*;
pub(crate) use fixtures::*;
pub(crate) use history::*;
pub(crate) use host::*;
pub(crate) use io::*;
pub(crate) use probe::*;
pub(crate) use proxy::*;
pub(crate) use registry::*;
pub(crate) use residents::*;
pub(crate) use standin::*;
pub(crate) use vm::*;
pub(crate) use workspace::*;
