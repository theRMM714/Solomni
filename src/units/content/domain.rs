//! 目的：纯逻辑——内容盘的落点派生。
//! 管：模块 id → 私有区的形状、`userdata/` 与 `build/` 两个子落点、运行包根。
//! 不管：介质名（在 `kernel::domain::layout`）；读写与存在性（在 service 与适配器）。
//! 联动：由 `api` 重导出给各调用方。

pub mod paths;
