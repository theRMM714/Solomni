//! 目的：向对象写授权与撤授权——授一条 ACE（标准 / 对象 ACE）或按 SID 撤掉已有 ACE。
//! 管：grant_one / grant_one_object_ace / write_ace / revoke_one 与它们对 SetEntriesInAclW 的拼装。
//! 不管：写后核对与失败回滚（verify.rs）、哪些落点要授（target.rs）。
//! 联动：windows/record 的按条撤销经 acl.rs 调到这里；落点由 prepare_fence 的 FenceSpec 决定。

use super::ace::{GUID_LEN, guid_from_bytes};
use super::*;
use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, OBJECTS_AND_SID, REVOKE_ACCESS,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_OBJECTS_AND_SID, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACE_OBJECT_TYPE_PRESENT, ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION,
    OBJECT_INHERIT_ACE, PSID, SID,
};

/// 目的：给一个对象授一条 ACE。`recursive` = 连**已有**子项一起设成这个 ACL（TreeSet）；`inherit` = 这条 ACE 被**新建**子项继承。
///   两者都要有明确理由：`inherit` 会牵动整棵子树的继承计算（真机实测：2000 个子项的可继承 ACE 写入是空目录的
///   20 倍），`recursive` 更是会把整棵树设一遍 ACL——所以只对**真的需要被子项继承**的落点（解释器目录、数据边界）用。
pub(crate) fn grant_one(
    sid: PSID,
    path: &Path,
    rights: u32,
    recursive: bool,
    inherit: bool,
) -> Result<(), String> {
    write_ace(sid, path, rights, recursive, inherit, None)
}

/// 目的：探针用的对象 ACE 入口——写一条**带类型 GUID** 的允许 ACE，用来钉住"写后核对覆盖对象 ACE"。
/// 参数：object_guid 是 ObjectType GUID 的 16 字节（按本机字节序，与 guid_text 的转储口径一致）。
/// 约束：只在测试构建里存在——产品路径不会写对象 ACE（写它只在探针里有意义）。
#[cfg(test)]
pub(crate) fn grant_one_object_ace(
    sid: PSID,
    path: &Path,
    rights: u32,
    object_guid: [u8; GUID_LEN],
) -> Result<(), String> {
    write_ace(sid, path, rights, false, false, Some(object_guid))
}

/// 目的：把一条授权写进对象的 DACL——标准 ACE，或者（给了 object_guid 时）**对象 ACE**。
/// 参数：recursive = 连已有子项一起设（TreeSet）；inherit = 这条 ACE 被新建子项继承；object_guid 决定 ACE 形态。
/// 返回：写入成功给 Ok(())，否则给出"哪一步、哪条路径、什么错"。
/// 错误：读 DACL、拼 ACL、写 DACL 任一步失败都如实返回；调用方据此回滚。
/// 约束：对象 ACE 的 SID 前面多一个 Flags(4) 与类型 GUID——SetEntriesInAclW 按 trustee 形态自己拼，
///   所以读回来时必须用 ace_parts 算出的偏移解析（这正是本模块两处口径要对齐的地方）。
pub(super) fn write_ace(
    sid: PSID,
    path: &Path,
    rights: u32,
    recursive: bool,
    inherit: bool,
    object_guid: Option<[u8; GUID_LEN]>,
) -> Result<(), String> {
    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("读 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let mut objects = OBJECTS_AND_SID {
        ObjectsPresent: ACE_OBJECT_TYPE_PRESENT,
        ObjectTypeGuid: guid_from_bytes(object_guid.unwrap_or([0; GUID_LEN])),
        InheritedObjectTypeGuid: guid_from_bytes([0; GUID_LEN]),
        pSid: sid as *mut SID,
    };
    let trustee = match object_guid {
        Some(_) => TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_OBJECTS_AND_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: &mut objects as *mut OBJECTS_AND_SID as *mut u16,
        },
        None => TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    };
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: rights,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: if inherit {
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
        } else {
            0
        },
        Trustee: trustee,
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let rc = unsafe { SetEntriesInAclW(1, &ea, old_dacl as *const ACL, &mut new_dacl) };
    if rc != 0 || new_dacl.is_null() {
        unsafe {
            LocalFree(sd);
        }
        return Err(format!("拼 ACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let rc = if recursive {
        unsafe {
            TreeSetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT as u32,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
                TREE_SEC_INFO_SET,
                std::ptr::null_mut(),
                PROGRESS_INVOKE_NEVER,
                std::ptr::null_mut(),
            )
        }
    } else {
        unsafe {
            SetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl as *const ACL,
                std::ptr::null(),
            )
        }
    };
    unsafe {
        LocalFree(new_dacl as *mut c_void);
        LocalFree(sd);
    }
    if rc != 0 {
        return Err(format!("写 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    Ok(())
}

/// 目的：把该 SID 的 ACE 从对象上撤掉（会话删除时清理用）。
pub(crate) fn revoke_one(sid: PSID, path: &Path, recursive: bool) -> Result<(), String> {
    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("读 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: 0,
        grfAccessMode: REVOKE_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let rc = unsafe { SetEntriesInAclW(1, &ea, old_dacl as *const ACL, &mut new_dacl) };
    if rc != 0 {
        unsafe {
            LocalFree(sd);
        }
        return Err(format!(
            "拼撤销后的 ACL 失败（{}）：错误码 {}",
            path.display(),
            rc
        ));
    }
    let rc = if recursive {
        unsafe {
            TreeSetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT as u32,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
                TREE_SEC_INFO_SET,
                std::ptr::null_mut(),
                PROGRESS_INVOKE_NEVER,
                std::ptr::null_mut(),
            )
        }
    } else {
        unsafe {
            SetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl as *const ACL,
                std::ptr::null(),
            )
        }
    };
    unsafe {
        if !new_dacl.is_null() {
            LocalFree(new_dacl as *mut c_void);
        }
        LocalFree(sd);
    }
    if rc != 0 {
        return Err(format!(
            "写撤权后的 DACL 失败（{}）：错误码 {}",
            path.display(),
            rc
        ));
    }
    Ok(())
}
