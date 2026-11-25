//! 目的：会话盘的入站面——别的单元与组合根只准用这里。
//! 管：`Sessions`（会话根 + 会话目录存在性）。
//! 不管：目录内部的布局、文件名与格式（在 `conversation` / `workspace`）；路径常量（由组合根经 `layout::session` 给）。
//! 联动：实现见 `service.rs`；`system` 的 `work_gc` 消费 `exists`。

use std::path::PathBuf;

/// 目的：会话盘的唯一读写面。
/// 约束：`root` 只交给组合根去构造两个文件适配器（`FsHistory` / `FsWorkspace`），业务 service 不拿路径（R16）。
pub trait Sessions: Send + Sync {
    /// 目的：会话目录的父目录（`data/session`）——转录与共享区共用会话目录 `<root>/<会话名>/`，各写自己的子路径。
    fn root(&self) -> PathBuf;

    /// 目的：这个会话名在这块盘上有没有目录；业务层问事实，不自己碰文件系统。
    fn exists(&self, name: &str) -> bool;
}
