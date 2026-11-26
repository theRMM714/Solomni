//! 目的：会话身份与声明的入站面——别的单元与呈现层只准用这里。
//! 管：重导出会话元信息与历史视图、会话类型表与解析后声明、机制与席位表、落盘策略。
//! 不管：会话对象与回合（在会话单元）；机制与工具总表的自洽断言（在会话单元的 `seat_check`，仍经会话单元导出）。
//! 联动：会话单元、work、turn、runtime、presentation 与测试都经这里取会话身份的形状。

#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::history::{
    Delegation, HistoryView, RunState, SessionMeta, UsageTotals, WorkspaceBinding,
    default_freshness_keep_percent,
};
#[cfg(test)]
pub use crate::units::business::session_model::domain::mechanism::PROXY_SEAT;
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::mechanism::{
    MECHANISMS, Mechanism, SEATS, Seat, flow_ids, mechanism_of, seat,
};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::persist::{
    PersistPolicy, SYSTEM_SID_SUGGEST, is_system_session, persist_policy_for,
};
#[allow(unused_imports)]
pub use crate::units::business::session_model::domain::profile::{
    Profiles, SessionSpec, inline_spec,
};
// 只有测试知道产品自带类型的名字；生产代码按声明里的 auto / flows 分支，不认这些 id。
#[cfg(test)]
pub use crate::units::business::session_model::domain::profile::{CUSTOM, MEETING, PROXY, SINGLE};
