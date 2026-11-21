//! 目的：Windows 安全描述符操作——逐路径改 DACL（授予 / 撤销）、解析 ACE 与 SID、展开泛型掩码、读强制完整性标签。
//! 管：ACE 布局与身份、授权落点清单、ACE 写入与撤销、DACL 读数与还原、写后核对与在场判断、写 DACL 的资格预检、
//!   完整性标签读数与挡写判据、解释器目录的基线判据；权限位常量与 TreeSetNamedSecurityInfoW 声明也只在这里。
//! 不管：围栏策略（哪些根、什么权限）——由调用方传入的 FenceSpec 决定；容器身份（在 `container`）；台账（在 `ledger`）。
//! 联动：由 `confine::prepare_fence` 逐条授权、由 `ledger` 的收尾撤销；容器 SID 派生在 `container`，两处共用同一批权限位。

use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Security::{ACL, PSID};

mod access;
mod ace;
mod grant;
mod label;
mod scan;
mod sid;
mod target;
#[cfg(test)]
mod tests;
mod verify;

pub(crate) use access::*;
pub(crate) use ace::*;
pub(crate) use grant::*;
pub(crate) use label::*;
pub(crate) use scan::*;
pub(crate) use sid::*;
pub(crate) use target::*;
pub(crate) use verify::*;

/// 目的：围栏描述符（`FenceSpec` / `FencePart`）——授权落点清单按它算，入参形状来自契约。
pub(crate) use crate::kernel::contracts::fence::{FencePart, FenceSpec};
/// 目的：容器身份（`container`）的那批符号——ACL 层要按它派生 SID、转 UTF-16、放行解释器基线。
pub(crate) use crate::kernel::detail::container::*;

/// 目的：文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
pub(crate) const SE_FILE_OBJECT: i32 = 1;
// 权限位**只用具体位**：通用位（GENERIC_*）的常量值极易记错，写错一个给出去的就是完全不同的权限——
// 标着 GENERIC_READ 的其实是 GENERIC_WRITE、标着 GENERIC_EXECUTE 的其实是 GENERIC_ALL。
pub(crate) const FILE_GENERIC_READ: u32 = 0x0012_0089;
pub(crate) const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
pub(crate) const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
pub(crate) const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
/// 目的：目录里建/删子项（工具要能重写自己的产物）。
pub(crate) const FILE_DELETE_CHILD: u32 = 0x0000_0040;
pub(crate) const DELETE: u32 = 0x0001_0000;
pub(crate) const WRITE_DAC: u32 = 0x0004_0000;
/// 目的：读安全描述符的权限位（AccessCheck 判写 DACL 时一起要）。
pub(crate) const READ_CONTROL: u32 = 0x0002_0000;
/// 目的：通用位：ACL 里存的可能是它们，也可能是内核展开后的具体位，比较覆盖关系时两者等价。
pub(crate) const GENERIC_READ: u32 = 0x8000_0000;
pub(crate) const GENERIC_WRITE: u32 = 0x4000_0000;
pub(crate) const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub(crate) const GENERIC_ALL: u32 = 0x1000_0000;

/// 目的：数据边界（会话目录、模块目录）→ 读写 + 删子项 + 写 DACL（撤权要用）。
pub(crate) const RIGHTS_RW: u32 = FILE_GENERIC_READ
    | FILE_GENERIC_WRITE
    | FILE_GENERIC_EXECUTE
    | FILE_DELETE_CHILD
    | DELETE
    | WRITE_DAC;
/// 目的：只读 + 执行（解释器安装目录：脚本要跑就得读得到它）。
pub(crate) const RIGHTS_RO: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
/// 目的：只读属性：**能判断"这个目录在不在"**，但读不到内容、列不了目录。
///   数据边界的父目录只授这一位（理由见 grant_targets）。
pub(crate) const RIGHTS_STAT: u32 = 0x0000_0080;

/// 目的：TREE_SEC_INFO_SET（递归设置）。
pub(crate) const TREE_SEC_INFO_SET: u32 = 1;
/// 目的：ProgressInvokeNever（不回调进度）。
pub(crate) const PROGRESS_INVOKE_NEVER: u32 = 1;

// SAFETY: 只在 Windows 后端调用 advapi32 的导出函数；参数由本文件的 Dacl 构造保证有效。
unsafe extern "system" {
    /// 递归给整棵树设 DACL：SDK 头文件在部分版本里标了废弃，但 advapi32 始终导出（本机已实测）。
    fn TreeSetNamedSecurityInfoW(
        object_name: *const u16,
        object_type: u32,
        security_info: u32,
        owner: PSID,
        group: PSID,
        dacl: *mut ACL,
        sacl: *mut ACL,
        action: u32,
        progress: *mut c_void,
        invoke: u32,
        args: *mut c_void,
    ) -> u32;
}

/// 目的：解释器目录上是不是已经有够用的只读基线 ACE——**两个主体都认**：
///   本产品写的用能力 SID；系统目录（System32 等）自带的是 ALL APPLICATION PACKAGES，那不是我们写的、
///   也改不动，但它确实让容器读得到解释器，所以同样算放行。
/// 约束：只读探测，不写任何 DACL。
pub(crate) fn interpreter_baseline_present(dir: &Path) -> Result<bool, String> {
    let product = product_capability_sid().map_err(|e| format!("派生产品能力 SID 失败：{}", e))?;
    if has_ace_for(product.as_ptr() as PSID, dir, RIGHTS_RO) {
        return Ok(true);
    }
    let aapp = system_baseline_sid().map_err(|e| format!("派生共享包 SID 失败：{}", e))?;
    let has = has_ace_for(aapp, dir, RIGHTS_RO);
    free_sid(aapp);
    Ok(has)
}

/// 目的：解释器安装目录在 Windows 容器围栏下能不能放行。
///   已有足够的基线 ACE（能力 SID 或系统自带的全应用包）就不用写；否则只有属主是当前用户（能写 DACL）才算可用。
/// 约束：只读探测；真正的递归授权仍由 prepare_fence 做，失败信息以那里为准。
pub(crate) fn interpreter_dir_usable(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("目录不在场（{}）", dir.display()));
    }
    if interpreter_baseline_present(dir)? {
        return Ok(());
    }
    match owner_is_current_user(dir) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => return Err(format!("读取目录属主失败：{}", e)),
    }
    match can_write_dacl(dir) {
        Ok(true) => Ok(()),
        Ok(false) => Err(
            "目录属主不是当前用户，当前用户也没有写 DACL 的权限；写 DACL 需要管理员，或预先把该目录授给本产品的能力 SID"
                .to_string(),
        ),
        Err(e) => Err(format!("判定写 DACL 权限失败：{}", e)),
    }
}
