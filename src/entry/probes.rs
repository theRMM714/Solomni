//! 目的：机器可读的内部探针——守门子进程、协议验证与路由目录；测试与 CI 的接口，不是用户功能。
//! 管：内部协议模式的入口与事实输出（不进命令表，也不列进 --help）。
//! 不管：产品命令（在 systools/commands.yaml + system 能力）、能力机制与装配（在各能力 detail 与 entry::assembly）。
//! 联动：由 `src/main.rs` 经 `entry::args` 在取根前/取根后调用；能力面装配统一在 `entry::assembly`。

pub mod fence;
pub mod https;
pub mod routes;
pub mod unpack;

pub use fence::{fence_prepare, fence_verify, print_fence_env};
pub use https::https_check;
pub use routes::print_routes;
pub use unpack::unpack_cli;
