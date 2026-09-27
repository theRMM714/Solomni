//! 工作区的**用例与端口持有者**：三个出站端口只在这里（R12）——
//! 清单来源、运行包库来源、目录布局。别的能力要清单事实或工作区目录，走 `api::Workspace`。
//!
//! 装配（new 出适配器）在组合根；这里只收注入的端口。

use crate::capabilities::workspace::api::{Library, Roster, WorkFiles, WorkRoots, Workspace};
use crate::capabilities::workspace::ports::{ModuleSource, PackageSource, Workdirs};
use std::sync::Arc;

/// 工作区能力：持三个端口，按用例答话。
pub struct WorkspaceService {
    source: Arc<dyn ModuleSource + Send + Sync>,
    packages: Arc<dyn PackageSource + Send + Sync>,
    dirs: Arc<dyn Workdirs + Send + Sync>,
}

impl WorkspaceService {
    /// 组合根专用。
    pub fn new(
        source: Arc<dyn ModuleSource + Send + Sync>,
        packages: Arc<dyn PackageSource + Send + Sync>,
        dirs: Arc<dyn Workdirs + Send + Sync>,
    ) -> WorkspaceService {
        WorkspaceService {
            source,
            packages,
            dirs,
        }
    }
}

impl Workspace for WorkspaceService {
    fn roster(&self) -> Roster {
        self.source.scan()
    }

    fn library(&self) -> Library {
        self.packages.scan()
    }

    fn runtimes_dir(&self) -> std::path::PathBuf {
        self.packages.dir()
    }

    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.prepare(session, agents)
    }

    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.dirs.write_work(session, name, bytes)
    }

    fn work_has(&self, session: &str, name: &str) -> bool {
        self.dirs.work_has(session, name)
    }

    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String> {
        self.dirs.list(session, agents)
    }

    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String> {
        self.dirs.roots(session, agents)
    }
}
