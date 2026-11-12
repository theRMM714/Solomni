//! 目的：测试替身与测试组合根（doubles）——内存适配器、共享夹具与装配辅助。
//! 管：各出站端口的内存实现、测试用能力面工厂、核心装配变体；本模块对“只有用例才用到”的项放行 dead_code。
//! 不管：用例本身（各业务测试文件）、真实适配器（`src/capabilities/*/detail/`）。
//! 联动：语义规范见 docs/testing/doubles.md；端口契约测试与它同处一层（`src/tests/`）。

#![allow(dead_code)]

use super::builders::SilentRunner;
use crate::capabilities::conductor::api::{AgentInstance, SessionEvent, WorkMode, WorkSpec};
use crate::capabilities::conductor::service::Conductor;
use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};
use crate::capabilities::llm::detail::fake_chat::FakeChat;
use crate::capabilities::llm::ports::{ChatGateway, ModelCatalog};
use crate::capabilities::modules::api::{Module, ModuleManifest};
use crate::capabilities::modules::ports::ModuleSource;
use crate::capabilities::packages::domain::manifest::{Library, PackageManifest};
use crate::capabilities::packages::ports::PackageSource;
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::prompt::domain::prompt::Prompts;
use crate::capabilities::prompt::ports::PromptSource;
use crate::capabilities::registry::api::{ModelEntry, Provider, Settings};
use crate::capabilities::registry::ports::SettingsStore;
use crate::capabilities::session::api::Live;
use crate::capabilities::session::api::{HistoryView, SessionMeta};
use crate::capabilities::session::ports::HistoryStore;
use crate::capabilities::tools::ports::SystoolsSource;
use crate::capabilities::tools::ports::{FileRead, SysIo};
use crate::capabilities::workspace::domain::asset::{AssetScanner, FileVersion};
use crate::capabilities::workspace::ports::{WorkStore, Workdirs};
use crate::kernel::api::Tier;
use crate::kernel::ports::ProcessRunner;
use std::collections::BTreeMap;
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
pub(crate) use vm::*;
pub(crate) use workspace::*;
