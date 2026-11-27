//! 目的：会话身份与声明的入站面——别的单元与呈现层只准用这里。
//! 管：重导出权限链派生、流程交接的产物与种类、会话元信息与历史视图、声明聚合、会话类型表与解析后声明、流程段与角色表、落盘策略、工具来源端口与回合上下文、裁决通道的发起面、工作台声明。
//! 不管：会话对象与回合（在会话单元）；段与工具总表的自洽断言（在会话单元的 `loop_check`，仍经会话单元导出）；声明装载（在 `detail`）。
//! 联动：会话单元、work、turn、runtime、presentation 与测试都经这里取会话身份的形状。

#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::access::permissions;
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::artifact::{Artifact, Kind};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::decisions::AskDesk;
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::decls::SessionDecls;
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::history::{
    Delegation, HistoryView, RunState, SessionMeta, UsageTotals, default_freshness_keep_percent,
};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::mechanism::{Cast, Loop, Loops, Role};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::persist::{
    PersistPolicy, SYSTEM_SID_SUGGEST, is_system_session, persist_policy_for,
};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::profile::{Profiles, SessionSpec};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::tools::{
    EXEC, FS_READ, FS_WRITE, SHARED_WRITE, SOURCE_AGENT, SOURCE_LOOP, SOURCE_MODULE,
    SOURCE_PROFILE, SOURCE_WORKSPACE, ToolSource, TurnCtx,
};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::workspace::{LAYOUT_POLICY, WorkspaceDecl};
// 只有测试知道产品自带类型的名字；生产代码按声明里的 auto / flows 分支，不认这些 id。
#[cfg(test)]
pub use crate::units::business::session_model::domain::profile::{MEETING, PROXY, SINGLE};
