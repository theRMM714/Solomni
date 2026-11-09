//! 目的：**存储空间**（业务能力）——可回收缓存的盘点、预算、自动/手动回收与显式清理。
//! 管：构建缓存卷、依赖下载缓存、过期可写层、日志的体积与回收；虚拟根还原入口；空间报告与作业。
//! 不管：构建怎么跑（在 builds）、guest 怎么起（在 vm）、下载怎么装（在 provision）、工作对象可达性（在 workspace）。
//! 联动：由组合根装配并注入 Ops；构建能力在起 guest 前问它要 headroom；口径见 docs/storage/README.md。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
