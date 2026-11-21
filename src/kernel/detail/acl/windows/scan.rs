//! 目的：读一个对象的安全描述符——逐条 ACE 读数、权限项身份集合、整份 SD 字节的保存与还原。
//! 管：acl_scan / acl_entries / permission_entries / lost_entries / sd_bytes / restore_sd / orphan_package_aces / is_sid_text。
//! 不管：ACE 布局与身份口径（ace.rs）、写权限项（grant.rs）、写后核对（verify.rs）。
//! 联动：windows/record 的清单与收尾经 acl.rs 读现值；verify.rs 用同一处读数口径做写后核对。

use super::ace::{ACCESS_ALLOWED_ACE_TYPE, ACE_FIXED_LEN};
use super::*;
use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::{
    ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, GetAce, GetAclInformation,
    GetSecurityDescriptorLength, PSECURITY_DESCRIPTOR, PSID, SetFileSecurityW,
};

/// 目的：读出一个对象的全部 ACE（允许 / 拒绝、标准 / 对象 / 回调），带类型、标志、SID、权限位与对象 GUID。
/// 返回：按 DACL 顺序排列的读数 + 认不出来的条数（不静默吞掉布局不认识的 ACE）。
/// 错误：读不到对象 DACL 时返回原因。
pub(crate) fn acl_scan(path: &Path) -> Result<DaclScan, String> {
    const ACL_SIZE_INFORMATION_CLASS: i32 = 2;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("读 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let mut entries: Vec<AceView> = Vec::new();
    let mut unparsed = 0usize;
    if !dacl.is_null() {
        let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            GetAclInformation(
                dacl,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                ACL_SIZE_INFORMATION_CLASS,
            )
        };
        if ok != 0 {
            for i in 0..info.AceCount {
                let mut ace: *mut c_void = std::ptr::null_mut();
                if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
                    unparsed += 1;
                    continue;
                }
                let base = ace as *const u8;
                // AceSize 在头部第 2、3 字节：按它切出这条 ACE 的字节，既不读别人的内存也不自己猜长度。
                let size = unsafe { u16::from_le_bytes([*base.add(2), *base.add(3)]) } as usize;
                if size < ACE_FIXED_LEN {
                    unparsed += 1;
                    continue;
                }
                let bytes = unsafe { std::slice::from_raw_parts(base, size) };
                match ace_parts(bytes) {
                    Some(parts) => entries.push(AceView {
                        ace_type: parts.ace_type,
                        flags: parts.flags,
                        mask: parts.mask,
                        sid: sid_to_string(unsafe { base.add(parts.sid_offset) as PSID }),
                        object_type: parts.object_type,
                        inherited_object_type: parts.inherited_object_type,
                    }),
                    None => unparsed += 1,
                }
            }
        }
    }
    unsafe {
        LocalFree(sd);
    }
    Ok(DaclScan { entries, unparsed })
}

/// 目的：读出一个对象的**权限项**多重集（允许 / 拒绝，含对象与回调 ACE），供探针与写后核对用同一个口径比对。
/// 返回：已排序的身份元素（ACE 类型, SID, 权限位, 对象类型 GUID, 继承对象类型 GUID）；继承与显式标志被忽略。
/// 错误：读不到对象 DACL 时返回原因。
/// 约束：只有探针用得到它——产品路径上的写后核对自己调 acl_scan（同一次读数里还要报"认不出的条数"），
///   所以它只在测试构建里存在，免得非测试构建留下一个没人用的入口。
#[cfg(test)]
pub(crate) fn acl_entries(path: &Path) -> Result<Vec<AceEntry>, String> {
    Ok(permission_entries(&acl_scan(path)?))
}

/// 目的：一次 DACL 读数里参与比对的身份集合（已排序）——读盘与写后核对共用同一处口径。
pub(super) fn permission_entries(scan: &DaclScan) -> Vec<AceEntry> {
    let mut out: Vec<AceEntry> = scan
        .entries
        .iter()
        .filter(|v| is_permission_ace(v.ace_type))
        .map(entry_of)
        .collect();
    out.sort();
    out
}

/// 目的：找出「写前在场、写后整个不见了」的不同权限项身份（集合语义）。
/// 约束：TreeSet 会把子节点上重复的 ACE 收敛成一份，出现次数变少不算丢失；同一身份仍在即算在场。
pub(crate) fn lost_entries(before: &[AceEntry], after: &[AceEntry]) -> Vec<AceEntry> {
    let mut lost = Vec::new();
    for item in before {
        if !after.contains(item) && !lost.contains(item) {
            lost.push(item.clone());
        }
    }
    lost
}

/// 目的：读出一个对象的 DACL 安全描述符（self-relative 字节），供收尾整体还原。
/// 错误：读不到对象 DACL 时返回原因。
pub(crate) fn sd_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("读 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let bytes = unsafe {
        let len = GetSecurityDescriptorLength(sd) as usize;
        std::slice::from_raw_parts(sd as *const u8, len).to_vec()
    };
    unsafe {
        LocalFree(sd);
    }
    Ok(bytes)
}

/// 目的：把 sd_bytes 记下的安全描述符整体写回对象（回滚与收尾还原）。
/// 错误：写回被拒时返回原因。
pub(crate) fn restore_sd(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let w = wide(path);
    let rc = unsafe {
        SetFileSecurityW(
            w.as_ptr(),
            DACL_SECURITY_INFORMATION,
            bytes.as_ptr() as PSECURITY_DESCRIPTOR,
        )
    };
    if rc == 0 {
        return Err(format!(
            "写回安全描述符失败（{}）：{}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// 目的：列出对象上显式（非继承）的、**本产品可能写下**的允许 ACE，供台账外孤儿授权的回收。
/// 返回：SID 字符串——每 agent 的容器 SID（S-1-15-2-*，排除 ALL APPLICATION PACKAGES 与 ALL RESTRICTED
///   两个系统基线），以及本产品的能力 SID（S-1-15-3-*，只认自己派生出的那一个，不碰别的软件写的能力 ACE）。
/// 错误：读不到对象 DACL 时返回原因。
pub(crate) fn orphan_package_aces(path: &Path) -> Result<Vec<String>, String> {
    let product = product_capability_sid()
        .ok()
        .map(|sid| sid_to_string(sid.as_ptr() as PSID));
    Ok(acl_scan(path)?
        .entries
        .into_iter()
        .filter(|v| {
            if v.ace_type != ACCESS_ALLOWED_ACE_TYPE || v.flags & INHERITED_ACE != 0 {
                return false;
            }
            if product.as_deref() == Some(v.sid.as_str()) {
                return true;
            }
            v.sid.starts_with("S-1-15-2-") && v.sid != "S-1-15-2-1" && v.sid != "S-1-15-2-2"
        })
        .map(|v| v.sid)
        .collect())
}

/// 目的：这段文本是不是一个真的 SID——转换失败时 sid_to_string 会给占位文本，它不能当判等依据。
pub(super) fn is_sid_text(text: &str) -> bool {
    text.starts_with("S-1-")
}
