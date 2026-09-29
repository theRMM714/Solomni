//! **细节实现 = 本能力自己的适配器**：只能由**入口层的组合根**构造（门禁判定）。

pub mod confine;
pub mod proc_tools;
pub mod sys_io;
pub mod yaml_systools;

pub use proc_tools::ProcTools;
pub use sys_io::FsSysIo;
