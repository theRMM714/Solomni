//! 目的：工作台契约单元的入站面——其它单元与呈现层只准用这里。
//! 管：一席工作面的形状与寻址语义（`Area` / `Bench` / `Benches`）、寻址根与用量（`WorkRoots` / `WorkFiles` /
//!   `AreaUsage` / `WorkUsage` / `safe_file_name`）、过滤名单（`filter`）、版本身份与门禁端口（`gate` / `readfact`）、
//!   观察事实（`observed`）与工作台策略契约（`policy`）。
//! 不管：实现（`workspace` 的布局、`versions` 的版本库）；装配（组合根）。
//! 联动：`tools` 按 `Bench` 定位读写、按 `WriteGate` 判写；`agentloop` 持 `Bench` 并实现门禁。

#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::area::Area;
#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::bench::Bench;
#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::benches::Benches;

/// 目的：工作区过滤的唯一名单与匹配规则——`tools` 的 grep 与内容面的导入 / 清单都只从这里取。
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::business::workbench::domain::filter;
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::business::workbench::domain::filter::{
    MAX_RULE_BYTES, MAX_RULE_CHARS, MAX_RULE_LINES, Rules, WORKIGNORE, WORKIGNORE_SEED, check_rule,
};

#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::gate::{
    GateFault, GateItem, Need, VersionRef, WriteGate,
};
/// 目的：一次读的读取事实——门禁与过期判定都按它记账。
#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::readfact::{ReadFact, ReadQuery};

#[allow(unused_imports)]
// 入站面：观察事实的契约形状（工具记它、门禁按它判）
pub use crate::units::business::workbench::domain::observed::{
    Coverage, LineRange, OBSERVATION_FORMAT, ObservationFact, ObservationSource, normalize_ranges,
};

#[allow(unused_imports)]
// 入站面：策略表与回合钩子（组合根注册、会话装配、引擎执行）
pub use crate::units::business::workbench::domain::policy::{
    BatchDone, BenchDriver, BenchPlan, BenchPolicies, BenchPolicy, CallRequest, GateRequest,
    PLAIN_POLICY, PlanRequest, Policies, SHARED_TOOLS, VERSIONED_POLICY, VERSIONS_CAP, WriteDone,
};

#[allow(unused_imports)]
pub use crate::units::business::workbench::domain::roots::{
    AreaUsage, WorkFiles, WorkRoots, WorkUsage, safe_file_name,
};
