//! 目的：会话盘的用例实现——把"会话名"映射成会话目录，再把事实问题交给端口。
//! 管：会话根与会话目录的拼法、`Sessions` 的逐条实现。
//! 不管：文件系统原语（在 `ports::SessionDirs`）；会话目录内部布局（在消费方 detail）。
//! 联动：组合根给会话根与适配器；`system` 经 `exists` 判工作是否存在。

use std::path::PathBuf;

use crate::units::resource::sessions::api::Sessions;
use crate::units::resource::sessions::ports::SessionDirs;

/// 目的：会话盘单元的唯一实现。
pub struct SessionsService {
    root: PathBuf,
    dirs: Box<dyn SessionDirs>,
}

impl SessionsService {
    /// 目的：按组合根给的会话根与目录适配器装一次。
    pub fn new(root: PathBuf, dirs: Box<dyn SessionDirs>) -> SessionsService {
        SessionsService { root, dirs }
    }
}

impl Sessions for SessionsService {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn exists(&self, name: &str) -> bool {
        self.dirs.is_dir(&self.root.join(name))
    }
}
