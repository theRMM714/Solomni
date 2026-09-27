//! 适配层：实现各能力定义的端口。
//! 依赖方向：adapters → 各能力的端口（只依赖端口与数据结构），可引用外部库（ureq/serde_yaml）。
//! 本层不做装配决策；new 出来的实例由 main 组合根注入 Conductor。

pub mod host_probe;
pub mod log;
pub mod root;

pub use host_probe::HostProbeAdapter;
pub use log::FileLog;
