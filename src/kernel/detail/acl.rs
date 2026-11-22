//! 目的：Windows 安全描述符操作——逐路径改 DACL（授予 / 撤销）、解析 ACE 与 SID、读强制完整性标签。
//! 管：ACE 布局与身份、授权落点清单、ACE 写入与撤销、DACL 读数与还原、写后核对、写 DACL 的资格预检、
//!   完整性标签读数与挡写判据、解释器目录的基线判据；权限位常量与 `TreeSetNamedSecurityInfoW` 声明也在这里。
//! 不管：围栏策略（哪些根、什么权限）——由调用方传入的 `FencePlan` 决定；容器身份（在 `container`）；台账（在 `ledger`）。
//! 联动：由 `confine::windows::prepare_fence` 逐条授权、由 `ledger` 的收尾撤销；本机只有 Windows 有 ACL，其余平台只答解释器目录可用。

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;
#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
pub(crate) use other::*;
