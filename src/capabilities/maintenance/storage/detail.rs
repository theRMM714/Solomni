//! 目的：存储空间的机制实现——目录扫描、qcow2 事实、磁盘余量、跨进程锁与回收状态。
//! 管：只被 storage::service 使用的 IO 细节与三个端口的实现；不解释业务策略。
//! 不管：什么该删（domain）、什么时候删（service）。
//! 联动：service 只经 ports.rs 消费；口径见 docs/storage/README.md。

pub mod guests;
pub mod lock;
pub mod paths;
pub mod qemu;
pub mod reclaim;
pub mod scan;
pub mod space;
pub mod state;
pub mod survey;
