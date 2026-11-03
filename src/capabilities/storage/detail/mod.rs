//! 目的：存储空间的机制实现——目录扫描、qcow2 事实、磁盘余量、跨进程锁与回收状态。
//! 管：只被 storage::service 使用的 IO 细节；不解释业务策略。
//! 不管：什么该删（domain）、什么时候删（service）。
//! 联动：service 组合这些机制；口径见 docs/storage/README.md。

pub mod lock;
pub mod qemu;
pub mod scan;
pub mod space;
pub mod state;
