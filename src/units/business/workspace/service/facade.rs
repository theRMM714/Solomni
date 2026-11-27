//! 目的：`Workspace` 用例面的唯一 trait 实现——目录布局与文件寻址直接转发到端口。
//! 管：入站布局用例到端口调用的映射（bind / ensure_layout / discard_layout / work_has / files / usage / roots / is_external / shared_lock_path / write_work）。
//! 不管：版本化用例（在 `versions` 单元）；落盘机制（detail）。
//! 联动：`api::Workspace` 的唯一实现就是本文件；组合根注入 `FsWorkspace`。

use super::*;
use crate::units::business::workspace::api::{WorkFiles, WorkRoots, WorkUsage, Workspace};
use std::path::Path;

impl Workspace for WorkspaceService {
    fn bind(&self, session: &str, root: Option<&Path>) -> Result<(), String> {
        self.dirs.bind(session, root)
    }

    fn ensure_layout(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.ensure_layout(session, agents)
    }

    fn discard_layout(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.discard_layout(session, agents)
    }

    fn work_has(&self, session: &str, name: &str) -> bool {
        self.dirs.work_has(session, name)
    }

    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String> {
        self.dirs.list(session, agents)
    }

    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String> {
        self.dirs.usage(session, agents)
    }

    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String> {
        self.dirs.roots(session, agents)
    }

    fn is_external(&self, session: &str) -> bool {
        self.dirs.is_external(session)
    }

    fn shared_lock_path(&self, session: &str) -> Result<Option<std::path::PathBuf>, String> {
        self.dirs.shared_lock_path(session)
    }

    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.dirs.write_work(session, name, bytes)
    }
}
