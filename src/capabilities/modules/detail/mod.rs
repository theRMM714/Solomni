//! 目的：模块能力的细节实现（本能力自己的适配器）。
//! 管：`FsModules`（扫描 `modules/`）。
//! 不管：端口形状（在 `ports.rs`）；清单契约与校验（在 `domain/`）。
//! 联动：只能由入口层的组合根构造（门禁判定）。
pub mod fs_modules;
pub mod yaml_module_state;
pub use fs_modules::FsModules;
pub use yaml_module_state::YamlModuleState;
