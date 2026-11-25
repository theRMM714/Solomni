//! 目的：lifecycle 单元的方法族入口——`Lifecycle` 的门面（字段、构造）与两段的公共形状。
//! 管：`Lifecycle` 的字段与构造。
//! 不管：回档三模式（`rewind.rs`）、列表 / 打开 / 删除收尾（`history.rs`）。
//! 联动：runtime 经 `api` 调用本单元；`Turn` / `RebuildDeps` 都是方法形参，本单元不持第二份 `Turn`。

mod history;
mod rewind;

/// 目的：删除收尾对象与撤权回调定义在 `history.rs`，测试经 `api` 取（生产路径按返回值用，不按名字取）。
#[cfg(test)]
pub(crate) use history::{ReleaseByLease, SessionDelete};

use crate::kernel::ports::Log;

use crate::units::conversation::session::api::{guest_scope, subtree_of, tree_order};
use crate::units::residents::api::ResidentOps;
use crate::units::session_model::api::{HistoryView, SessionMeta};
use crate::units::tools::api::ToolExec;
use crate::units::turn_state::api::History;
use crate::units::turn_state::api::{Session, Turn};
use crate::units::vm::api::Vm;
use crate::units::workspace::api::Workspace;
use std::sync::Arc;

/// 目的：**会话历史生命周期的用例面**：回档三模式、历史列表与只读打开、删除的报账与收尾。
///  与 `Core` 分开：这里只读会话与工作区的既有真相，不推进协作链、不发事件——时机归 runtime。
///  组合根在 `Core::new` 里造一份，与 runtime 共享同一批入站面（Arc 克隆，不新建实现）。
pub struct Lifecycle {
    /// 会话历史直连面（**不持它的端口**，R12）：读 / 写 / 删会话都走它。
    history: Arc<dyn History + Send + Sync>,
    /// 工作区用例面（**不持它的端口**，R12）：留档、删除与恢复的版本化走它。
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 工具执行面（**不持它的端口**，R12）：删除后按会话释放围栏授权走它。
    tools: Arc<dyn ToolExec + Send + Sync>,
    /// 虚拟机档用例面（**不持它的端口**，R12）：删除前停 guest 走它。
    vm: Arc<dyn Vm + Send + Sync>,
    /// 常驻服务的统一管理面（**不持它的端口**）：删除前回收常驻服务租约经它。
    residents: Arc<dyn ResidentOps + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
}

impl Lifecycle {
    /// 目的：组合根专用——与 `Core` 共享同一批入站面，不新建任何实现。
    pub fn new(
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        residents: Arc<dyn ResidentOps + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
    ) -> Lifecycle {
        Lifecycle {
            history,
            workspace,
            tools,
            vm,
            residents,
            log,
        }
    }
}
