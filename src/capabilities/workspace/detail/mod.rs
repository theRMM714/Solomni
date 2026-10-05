//! **细节实现 = 本能力自己的适配器**：只能由**入口层的组合根**构造（门禁判定）。

pub mod fs_modules;
pub mod fs_packages;
pub mod fs_workspace;
pub mod fs_workstore;

pub use fs_modules::FsModules;
pub use fs_packages::FsPackages;
pub use fs_workspace::FsWorkspace;
pub use fs_workstore::FsWorkStore;
