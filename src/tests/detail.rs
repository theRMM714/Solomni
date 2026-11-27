//! 目的：真实适配器边界用例的入口：按适配器族分文件，每个文件只钉一族落盘契约。
//! 管：子模块声明与各族的归属（文件/设置/模块与包/提示词/日志/HTTP）。
//! 不管：各族用例的细节；内存替身与端口契约（见 docs/testing/doubles.md）。
//! 联动：隔离根由 `tests::scratch` 提供；真实适配器清单见 docs/testing/doubles.md。

mod clock;
mod fs;
mod http;
mod logs;
mod modules;
mod profiles;
mod prompt;
mod yaml;

use super::scratch;
use crate::kernel::detail::file_log::FileLog;
use crate::kernel::ports::{Log, NoopLog};
use crate::tests::doubles::assets_at;
use crate::units::business::catalog::modules::detail::fs_modules::FsModules;
use crate::units::business::catalog::modules::ports::ModuleSource;
use crate::units::business::catalog::packages::detail::FsPackages;
use crate::units::business::catalog::packages::ports::PackageSource;
use crate::units::business::catalog::registry::detail::yaml_settings::YamlSettingsStore;
use crate::units::business::catalog::registry::ports::SettingsStore;
use crate::units::business::conversation::session::detail::fs_history::FsHistory;
use crate::units::business::conversation::session::ports::HistoryStore;
use crate::units::business::llm::api::Channel;
use crate::units::business::llm::api::{Chunk, CompleteOpts, Msg, ProbeOutcome};
use crate::units::business::llm::detail::endpoint::memo_new;
use crate::units::business::llm::detail::http_chat::HttpGateway;
use crate::units::business::llm::detail::model_catalog::HttpModelCatalog;
use crate::units::business::llm::ports::{ChatGateway, ModelCatalog};
use crate::units::business::prompt::detail::yaml_prompts::YamlPrompts;
use crate::units::business::prompt::ports::PromptSource;
use crate::units::business::session_model::api::SessionMeta;
use crate::units::business::session_model::detail::yaml_decls::YamlDecls;
use crate::units::business::tools::detail::sys_io::FsSysIo;
use crate::units::business::tools::detail::yaml_systools::YamlSystools;
use crate::units::business::tools::ports::SysIo;
use crate::units::business::tools::ports::SystoolsSource;
use crate::units::business::versions::detail::fs_workstore::FsWorkStore;
use crate::units::business::versions::ports::WorkStore;
use crate::units::business::workspace::detail::fs_workspace::FsWorkspace;
use crate::units::business::workspace::ports::Workdirs;
use crate::units::pure::registry_model::api::AgentMeta;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
