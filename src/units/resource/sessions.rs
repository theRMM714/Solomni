//! 目的：会话盘单元——`data/session` 的目录所有权与"这个会话在不在盘上"的事实。
//! 管：会话根（转录与共享区两个适配器都从它派生）与会话目录的存在性查询。
//! 不管：会话目录内部的布局与文件格式（在 `conversation` 与 `workspace` 的 detail）；介质名与路径常量（在 `kernel` 的 `layout`）。
//! 联动：组合根用它构造 `FsHistory` / `FsWorkspace`；`system` 用它判"有没有这个工作"。

pub mod api;
pub mod detail;
pub mod ports;
pub mod service;
