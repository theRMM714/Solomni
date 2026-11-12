//! 目的：Windows 围栏的授权台账：一次执行动过哪些本机权限项，收尾时据此精确撤销。
//! 管：台账的唯一盘上格式与写事务、归属与活性、授权日志、撤销与回收、按条处置、启动对账——按职责分文件。
//! 不管：ACL 怎么读写与还原（acl.rs）、容器 SID 怎么派生（container.rs）。
//! 联动：windows.rs 的 prepare_fence / run_fenced 写账；confine.rs 的 release_by_lease 与 --fence-reconcile 按账收尾。

mod catalog;
mod journal;
mod liveness;
mod paths;
mod reconcile;
mod release;
mod sid;
mod store;
mod sweep;

pub(crate) use catalog::*;
pub(crate) use journal::*;
pub(crate) use liveness::*;
pub(crate) use paths::*;
pub(crate) use reconcile::*;
pub(crate) use release::*;
pub(crate) use sid::*;
pub(crate) use store::*;
pub(crate) use sweep::*;
