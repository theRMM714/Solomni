//! 目的：机制实现的装配面——kernel 的端口在这里落到具体实现。
//! 管：文件日志（`FileLog`）、主机事实探针（`HostProbeAdapter`）、外部进程与围栏（`confine` / `process`）。
//! 不管：端口定义（在 `ports`）；任何领域语义；配置从哪来（调用方传参）。
//! 联动：端口在 `src/kernel/ports.rs`；构造它们的唯一位置是入口层的组合根。

pub mod confine;
pub mod file_log;
pub mod host_probe;
pub mod process;

pub use file_log::FileLog;
pub use host_probe::HostProbeAdapter;
