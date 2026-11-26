//! 目的：测试替身与测试组合根（doubles）——内存适配器、共享夹具与装配辅助。
//! 管：各出站端口的内存实现、测试用入站面工厂、核心装配变体；本模块对“只有用例才用到”的项放行 dead_code。
//! 不管：用例本身（各业务测试文件）、真实适配器（`src/units/*/detail/`）。
//! 联动：语义规范见 docs/testing/doubles.md；端口契约测试与它同处一层（`src/tests/`）。

#![allow(dead_code)]

use super::builders::SilentRunner;
use crate::units::business::transcript::api::SessionEvent;
use crate::units::business::work::api::WorkSpec;
use crate::units::pure::registry_model::api::AgentMeta;

use crate::kernel::ports::ProcessRunner;
use crate::units::business::agentloop::api::TurnIo;
use crate::units::business::catalog::modules::api::{Module, ModuleManifest};
use crate::units::business::catalog::modules::ports::ModuleSource;
use crate::units::business::catalog::packages::domain::manifest::{Library, PackageManifest};
use crate::units::business::catalog::packages::ports::PackageSource;
use crate::units::business::catalog::registry::api::Settings;
use crate::units::business::catalog::registry::ports::SettingsStore;
use crate::units::business::conversation::session::ports::HistoryStore;
use crate::units::business::llm::api::Channel;
use crate::units::business::llm::api::{BoxedChat, Chat, Chunk, CompleteOpts, Completion, Msg};
use crate::units::business::llm::detail::fake_chat::FakeChat;
use crate::units::business::llm::ports::{ChatGateway, ModelCatalog};
use crate::units::business::prompt::api::Prompt;
use crate::units::business::prompt::domain::prompt::Prompts;
use crate::units::business::prompt::ports::PromptSource;
use crate::units::business::runtime::service::Core;
use crate::units::business::session_model::api::{HistoryView, SessionMeta};
use crate::units::business::tools::ports::SystoolsSource;
use crate::units::business::tools::ports::{FileRead, SysIo};
use crate::units::business::workspace::domain::asset::{AssetScanner, FileVersion};
use crate::units::business::workspace::ports::{WorkStore, Workdirs};
use crate::units::pure::exec_model::api::Tier;
use crate::units::pure::registry_model::api::ModelEntry;
use crate::units::pure::registry_model::api::Provider;
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

/// 目的：随包只读内容的**真实适配器**（指向给定安装根下的 `prompts/` 与 `systools/`）。
/// 参数：`root` = 产品安装根（仓库根，或用例造出来的隔离根）。
/// 返回：注入各装配输入适配器的 `Assets`。
pub(crate) fn assets_at(
    root: &std::path::Path,
) -> Arc<dyn crate::units::resource::assets::api::Assets> {
    Arc::new(crate::units::resource::assets::service::AssetsService::new(
        root,
        Box::new(crate::units::resource::assets::detail::FsAssetFs),
    ))
}

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
