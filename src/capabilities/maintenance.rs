//! 目的：maintenance 单元——构建产物与存储回收。
//! 管：builds 的产物构建、缓存与失效、storage 的空间统计、回收与虚拟根还原。
//! 不管：模块清单（catalog）、会话转录（conversation）、虚拟机运行（vm）。
//! 联动：单元内子业务各自保留四分区，跨单元引用经 maintenance::<子业务>::api（见 docs/builds/README.md、docs/storage/README.md）。
pub mod builds;
pub mod storage;
