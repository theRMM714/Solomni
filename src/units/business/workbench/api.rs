//! 目的：工作台契约单元的入站面——其它单元与呈现层只准用这里。
//! 管：重导出版本身份、门禁需求与拒绝原因、门禁端口。
//! 不管：实现（`versions` 的版本库）；装配（组合根）。
//! 联动：`tools` 的内置工具按 `WriteGate` 判写、`agentloop` 实现它。

#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::gate::{
    GateFault, GateItem, Need, VersionRef, WriteGate,
};
/// 目的：一次读的读取事实——门禁与过期判定都按它记账。
#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::readfact::ReadFact;
