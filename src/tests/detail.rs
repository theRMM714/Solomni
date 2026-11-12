//! 目的：真实适配器边界用例的入口：按适配器族分文件，每个文件只钉一族落盘契约。
//! 管：子模块声明与各族的归属（文件/设置/模块与包/提示词/日志/HTTP）。
//! 不管：各族用例的细节；内存替身与端口契约（见 docs/testing/doubles.md）。
//! 联动：隔离根由 `tests::scratch` 提供；真实适配器清单见 docs/testing/doubles.md。

mod fs;
mod http;
mod logs;
mod modules;
mod prompt;
mod yaml;

use super::scratch;
use crate::capabilities::llm::api::Channel;
use crate::capabilities::llm::api::{Chunk, CompleteOpts, Msg, ProbeOutcome};
use crate::capabilities::llm::detail::endpoint::memo_new;
use crate::capabilities::llm::detail::http_chat::HttpGateway;
use crate::capabilities::llm::detail::model_catalog::HttpModelCatalog;
use crate::capabilities::llm::ports::{ChatGateway, ModelCatalog};
use crate::capabilities::modules::detail::fs_modules::FsModules;
use crate::capabilities::modules::ports::ModuleSource;
use crate::capabilities::packages::detail::FsPackages;
use crate::capabilities::packages::ports::PackageSource;
use crate::capabilities::prompt::detail::yaml_prompts::YamlPrompts;
use crate::capabilities::prompt::ports::PromptSource;
use crate::capabilities::registry::api::{Provider, Settings};
use crate::capabilities::registry::detail::yaml_settings::YamlSettingsStore;
use crate::capabilities::registry::ports::SettingsStore;
use crate::capabilities::session::api::{AgentMeta, SessionMeta};
use crate::capabilities::session::detail::fs_history::FsHistory;
use crate::capabilities::session::ports::HistoryStore;
use crate::capabilities::tools::detail::sys_io::FsSysIo;
use crate::capabilities::tools::detail::yaml_systools::YamlSystools;
use crate::capabilities::tools::ports::SysIo;
use crate::capabilities::tools::ports::SystoolsSource;
use crate::capabilities::workspace::detail::fs_workspace::FsWorkspace;
use crate::capabilities::workspace::ports::Workdirs;
use crate::kernel::api::ExecSpec;
use crate::kernel::detail::file_log::FileLog;
use crate::kernel::ports::{Log, NoopLog};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
