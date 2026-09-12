//! 适配层：实现 core 定义的端口。
//! 依赖方向：adapters → core（只依赖端口与数据结构），可引用外部库（ureq/serde_yaml）。
//! 本层不做装配决策；new 出来的实例由 main 组合根注入 Core。

pub mod fs_modules;
pub mod log;
pub mod http_chat;
pub mod fake_chat;
pub mod yaml_prompts;
pub mod yaml_registry;

pub use fs_modules::FsModules;
pub use http_chat::HttpGateway;
pub use yaml_prompts::YamlPrompts;
pub use yaml_registry::YamlRegistryStore;
pub use log::FileLog;