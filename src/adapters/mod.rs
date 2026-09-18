//! 适配层：实现 core 定义的端口。
//! 依赖方向：adapters → core（只依赖端口与数据结构），可引用外部库（ureq/serde_yaml）。
//! 本层不做装配决策；new 出来的实例由 main 组合根注入 Core。

pub mod confine;
pub mod endpoint;
pub mod fake_chat;
pub mod fs_history;
pub mod fs_modules;
pub mod fs_packages;
pub mod fs_workspace;
pub mod http_agent;
pub mod http_chat;
pub mod log;
pub mod model_catalog;
pub mod proc_tools;
pub mod repair;
pub mod sys_io;
pub mod yaml_prompts;
pub mod yaml_settings;

pub use fs_history::FsHistory;
pub use fs_modules::FsModules;
pub use fs_packages::FsPackages;
pub use fs_workspace::FsWorkspace;
pub use http_chat::HttpGateway;
pub use log::FileLog;
pub use model_catalog::HttpModelCatalog;
pub use proc_tools::ProcTools;
pub use repair::UnambiguousRepair;
pub use sys_io::FsSysIo;
pub use yaml_prompts::YamlPrompts;
pub use yaml_settings::YamlSettingsStore;
