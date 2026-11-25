//! 目的：lifecycle 的入站面：回档目标与结果、重建要的两个能力面，以及删除收尾对象与撤权回调。
//! 管：类型的定义与文档（值的唯一家）。
//! 不管：怎么回档、怎么删（在 service 里）；呈现层的 JSON 封装。
//! 联动：runtime 与测试都从这里取类型；`SessionDelete::run()` 在调用方线程上跑。

use crate::units::catalog::registry::api::Registry;
use crate::kernel::ports::HostProbe;

/// 目的：回档目标：留档 / 删除按**行 id**，恢复按**留档标记 id**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindTarget {
    /// 目的：留档到某一行之前：标记 + 折叠，可恢复。
    Archive(u64),
    /// 目的：删除某一行及其后：真的截断，不可恢复。
    Delete(u64),
    /// 目的：恢复到某个留档标记之前：删掉该标记及其后的全部内容。
    Restore(u64),
}

/// 目的：重建会话要的两个能力面（`Turn::rebuild_session` 的入参）。
/// 约束：两者都按次传入、不当字段持有——`Turn` 不持登记处，本能力也不持第二份。
#[derive(Clone, Copy)]
pub(crate) struct RebuildDeps<'a> {
    pub(crate) registry: &'a dyn Registry,
    pub(crate) probe: &'a dyn HostProbe,
}

/// 目的：一次回档的完整结果——主会话活动视图 + 受影响的子会话活动视图。
/// 约束：主视图回给调用方；每份视图同时作为一条 `rewound` 推上事件台，供刷新与多端重置。
#[derive(Debug)]
pub(crate) struct RewindOutcome {
    pub main: Vec<serde_json::Value>,
    pub children: Vec<(String, Vec<serde_json::Value>)>,
}

// 类型定义在 service 里（删除收尾对象要拿能力面），对外经这里取：runtime 与测试只认这个面。
pub(crate) use crate::units::lifecycle::service::Lifecycle;
/// 目的：删除收尾对象与撤权回调——生产路径由 `delete_bookkeeping` 返回（不按名字取），测试直接构造。
#[cfg(test)]
pub(crate) use crate::units::lifecycle::service::{ReleaseByLease, SessionDelete};
