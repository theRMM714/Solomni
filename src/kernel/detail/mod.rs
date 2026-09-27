//! 机制实现（适配层）：只实现 `kernel/ports.rs` 的端口，不含任何领域语义。
//! **入口层（组合根）唯一**构造它们的地方。

pub mod file_log;
pub mod host_probe;

pub use file_log::FileLog;
pub use host_probe::HostProbeAdapter;
