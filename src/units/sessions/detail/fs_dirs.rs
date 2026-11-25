//! 目的：会话盘的真实目录适配器（实现 `ports::SessionDirs`）。
//! 管：`is_dir` 这一步宿主判定。
//! 不管：会话根与名字的拼接（在 service）；会话目录内部布局（在消费方 detail）。
//! 联动：组合根构造后注入 `service.rs`。

use std::path::Path;

use crate::units::sessions::ports::SessionDirs;

/// 目的：无状态的目录判定（会话根由 service 持有）。
pub struct FsSessionDirs;

impl FsSessionDirs {
    /// 目的：造一个适配器（无字段，组合根直接 `new`）。
    pub fn new() -> FsSessionDirs {
        FsSessionDirs
    }
}

impl Default for FsSessionDirs {
    fn default() -> Self {
        FsSessionDirs::new()
    }
}

impl SessionDirs for FsSessionDirs {
    fn is_dir(&self, dir: &Path) -> bool {
        dir.is_dir()
    }
}
