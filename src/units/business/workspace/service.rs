//! 目的：工作区单元的 service 门面——`WorkspaceService` 的字段与构造（目录布局与外部根策略）。
//! 管：`Workdirs` 端口只在这里（R12）；外部工作区根的策略函数（roots）。
//! 不管：版本库用例（在 `versions` 单元）；落盘实现（detail）。
//! 联动：`api::Workspace` 的唯一实现在 facade.rs；组合根注入 `FsWorkspace`。

use crate::units::business::workspace::ports::Workdirs;
use std::sync::Arc;

mod facade;
pub(crate) mod roots;

/// 目的：工作区单元——持目录布局端口，按布局用例答话。
pub struct WorkspaceService {
    dirs: Arc<dyn Workdirs + Send + Sync>,
}

impl WorkspaceService {
    /// 目的：组合根专用构造。
    pub fn new(dirs: Arc<dyn Workdirs + Send + Sync>) -> WorkspaceService {
        WorkspaceService { dirs }
    }
}
