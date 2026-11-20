//! 目的：Windows 安全描述符操作——逐路径改 DACL（授予 / 撤销）、解析 ACE 与 SID、展开泛型掩码、读强制完整性标签。
//! 管：ACE 布局与身份、授权落点清单、ACE 写入与撤销、DACL 读数与还原、写后核对与在场判断、写 DACL 的资格预检、完整性标签读数与挡写判据——按职责分文件，本文件只汇总导出。
//! 不管：围栏策略（哪些根、什么权限）——由调用方传入的 FenceSpec 决定。
//! 联动：授权台账与收尾在 record/、容器身份在 container.rs、外层调用在 confine::prepare_fence。

mod access;
mod ace;
mod grant;
mod label;
mod scan;
mod target;
#[cfg(test)]
mod tests;
mod verify;

pub(crate) use access::*;
pub(crate) use ace::*;
pub(crate) use grant::*;
pub(crate) use label::*;
pub(crate) use scan::*;
pub(crate) use target::*;
pub(crate) use verify::*;
