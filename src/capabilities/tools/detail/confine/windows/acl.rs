//! **安全描述符操作**：逐路径改 DACL（授予 / 撤销）、解析 ACE 与 SID、展开泛型掩码。
//!
//! 这些本该由内核替调用方算——Windows 上要自己算（`windows-sys` 未暴露 `TreeSetNamedSecurityInfoW`，ACE / ACL 结构也要自己认）。

use crate::capabilities::tools::api::FenceSpec;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorLength, SetFileSecurityW, ACL,
    ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
    PSECURITY_DESCRIPTOR, PSID,
};

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
use super::*;
/// 给一个对象授一条 ACE。`recursive` = 连**已有**子项一起设成这个 ACL（TreeSet）；`inherit` = 这条 ACE 被**新建**子项继承。
/// 两者都要有明确理由：`inherit` 会牵动整棵子树的继承计算（真机实测：2000 个子项的可继承 ACE 写入是空目录的
/// 20 倍），`recursive` 更是会把整棵树设一遍 ACL——所以只对**真的需要被子项继承**的落点（解释器目录、数据边界）用。
pub(crate) fn grant_one(
    sid: PSID,
    path: &Path,
    rights: u32,
    recursive: bool,
    inherit: bool,
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
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: rights,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: if inherit {
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
        } else {
            0
        },
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

/// 把该 SID 的 ACE 从对象上撤掉（会话删除时清理用）。
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

/// 命令里解释器的安装目录：共用实现在 confine/mod.rs（Windows 的目录 ACL 与 macOS 的 seatbelt 同一套语义）。
pub(crate) fn interpreter_dirs(command: &str) -> Vec<PathBuf> {
    super::super::interpreter_dirs(command)
}

/// 目的：一个授权落点（路径, 权限位, 是否递归整树, ACE 是否被子项继承）。
/// 约束：把 grant_targets 的元组收成一个名字，prepare / release / 回滚共用同一份形状。
pub(crate) type GrantTarget = (PathBuf, u32, bool, bool);

/// 围栏要授权的全部落点：数据边界叶子（读写 / 用户授权的只读）+ **它们的父目录**（只读属性）。
///
/// 父目录为什么要授：容器里对**中间目录**没有 FILE_READ_ATTRIBUTES 时，`exists()` / `stat()` 这类
/// 常规判断会对一个**确实存在**的目录返回假。后果不是"读不到"，而是模块的"父目录不存在就先建"逻辑
/// 以为整条链都不存在，一路向上建到盘卷根，撞出 `WinError 5 Access is denied: 'D:\'`
/// （真机 CI 上抓到的：Python 的 `os.makedirs(parent, exist_ok=True)`）。
///
/// 只授"读属性"、**不递归、不继承**：容器能判断存在性，但读不到内容、列不了目录。
/// 只到**直接父目录**为止（不是整条祖先链）：再往上就是产品根之外，而 stat 到直接父目录已足够让
/// "父目录在不在"这个判断成立。给祖先链授"穿过"要改写 `C:\` 这种巨型目录的 DACL
/// （顺整棵树重算继承，真机 ~90 s/条），这里授的是产品内的小目录，各一条 ACE。
pub(crate) fn grant_targets(spec: &FenceSpec) -> Vec<GrantTarget> {
    let mut todo: Vec<GrantTarget> = Vec::new();
    let mut leaves: Vec<GrantTarget> = Vec::new();
    for root in &spec.rw {
        if !root.as_os_str().is_empty() {
            leaves.push((root.clone(), RIGHTS_RW, true, true));
        }
    }
    // 用户显式授权的只读根（`fence_read`）：只写只读 ACE，**授给该 agent 自己的容器 SID**。
    // 不能像解释器基线那样授给共享组（S-1-15-2-1）——那等于把用户数据开放给机器上任意容器程序。
    // 只读根不递归：用户可能授一个很大的目录（例如项目根），递归会改整棵树的 DACL。
    for root in &spec.ro {
        if !root.as_os_str().is_empty() {
            leaves.push((root.clone(), RIGHTS_RO, false, true));
        }
    }
    // 只读**子树**（模块目录默认只读）：必须递归可读（工具脚本在目录里），所以 recursive + inherit。
    for root in &spec.ro_tree {
        if !root.as_os_str().is_empty() {
            leaves.push((root.clone(), RIGHTS_RO, true, true));
        }
    }
    // 工作目录（模块根）：工具进程要能在里面起（读 + 执行），但**不因此获得写**。
    // 已授权可写的模块同时也在 rw 里，那一条（更早入列）给的写权才是准的。
    if !spec.cwd.as_os_str().is_empty() {
        leaves.push((spec.cwd.clone(), RIGHTS_RO, true, true));
    }
    // 父目录：只读属性、不递归、**不继承**（元组末位是继承标志）。同一个父目录被多个叶子共用时
    // 靠调用方的去重表收口。不继承是为了把残留面收敛到父目录本身：带 (OI)(CI)
    // 的 ACE 会传播进已存在的子项、再传给之后新建的子项——一旦撤权断链（进程被杀、台账丢失），
    // 受污染的就是整棵子树；不继承把最坏残留面收敛到父目录本身，而新建子项反正会拿到自己的
    // 授权，不需要它。
    for (leaf, _, _, _) in &leaves {
        if let Some(parent) = leaf.parent() {
            if !parent.as_os_str().is_empty() {
                todo.push((parent.to_path_buf(), RIGHTS_STAT, false, false));
            }
        }
    }
    todo.extend(leaves);
    todo
}

/// 只读运行基线的授权对象：ALL APPLICATION PACKAGES（S-1-15-2-1）。
/// 我们的容器令牌本来就带这个组，所以「解释器与系统只读区」这类基线只授一次、与 agent 身份无关；
/// 每 agent 一个的容器 SID 只用来圈**数据边界**（会话目录、模块目录）。
pub(crate) fn baseline_sid() -> Result<PSID, String> {
    let s: Vec<u16> = std::ffi::OsStr::new("S-1-15-2-1")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(s.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err("取不到 ALL APPLICATION PACKAGES 的 SID".to_string());
    }
    Ok(sid)
}

/// 把通用位展开成具体位：ACL 里存的是哪一套，覆盖关系比较都要等价成立。
pub(crate) fn expand_generics(mask: u32) -> u32 {
    let mut out = mask;
    if mask & GENERIC_READ != 0 {
        out |= FILE_GENERIC_READ;
    }
    if mask & GENERIC_WRITE != 0 {
        out |= FILE_GENERIC_WRITE;
    }
    if mask & GENERIC_EXECUTE != 0 {
        out |= FILE_GENERIC_EXECUTE;
    }
    if mask & GENERIC_ALL != 0 {
        out |= FILE_ALL_ACCESS;
    }
    out
}

/// 已有 ACE 的权限位是不是覆盖得住我们需要的权限位。
pub(crate) fn rights_covered(mask: u32, rights: u32) -> bool {
    expand_generics(rights) & !expand_generics(mask) == 0
}

/// 目的：读出一个对象的全部标准 ACE（允许 / 拒绝），带类型、标志、SID 与权限位。
/// 返回：按 DACL 顺序排列的（ACE 类型, ACE 标志, SID 字符串, 权限位）。
/// 错误：读不到对象 DACL 时返回原因。
/// 约束：只覆盖标准 ACE（类型 0/1）；对象 ACE 的 SID 偏移不同，不在此列。
fn acl_scan(path: &Path) -> Result<Vec<(u8, u8, String, u32)>, String> {
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
    let mut out: Vec<(u8, u8, String, u32)> = Vec::new();
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
                    continue;
                }
                let base = ace as *const u8;
                let ace_type = unsafe { *base };
                let flags = unsafe { *base.add(1) };
                let mask = unsafe { std::ptr::read_unaligned(base.add(4) as *const u32) };
                let sid = unsafe { base.add(8) as PSID };
                out.push((ace_type, flags, sid_to_string(sid), mask));
            }
        }
    }
    unsafe {
        LocalFree(sd);
    }
    Ok(out)
}

/// 目的：读出一个对象的允许 / 拒绝 ACE 多重集，用于比对一次写入有没有弄丢原有权限项。
/// 返回：已排序的（ACE 类型, SID 字符串, 权限位）；继承与显式标志被忽略。
/// 错误：读不到对象 DACL 时返回原因。
pub(crate) fn acl_entries(path: &Path) -> Result<Vec<(u8, String, u32)>, String> {
    let mut out: Vec<(u8, String, u32)> = acl_scan(path)?
        .into_iter()
        .filter(|(ace_type, _, _, _)| *ace_type == 0 || *ace_type == 1)
        .map(|(ace_type, _, sid, mask)| (ace_type, sid, mask))
        .collect();
    out.sort();
    Ok(out)
}

/// 目的：找出「写前有、写后没了」的 ACE（多重集差）。
fn lost_entries(
    before: &[(u8, String, u32)],
    after: &[(u8, String, u32)],
) -> Vec<(u8, String, u32)> {
    let mut pool = after.to_vec();
    let mut lost = Vec::new();
    for item in before {
        match pool.iter().position(|x| x == item) {
            Some(pos) => {
                pool.remove(pos);
            }
            None => lost.push(item.clone()),
        }
    }
    lost
}

/// 目的：写一条授权并做写后核对（原有权限项不得丢失、我们的 ACE 必须生效）。
/// 参数：sid 是授权对象，path 是目标，rights 是权限位，recursive 与 inherit 同 grant_one。
/// 返回：写入且核对通过时 Ok(())。
/// 错误：读写 DACL 失败、原有权限项丢失、我们的 ACE 未生效，都如实返回；调用方据此回滚。
pub(crate) fn grant_verified(
    sid: PSID,
    path: &Path,
    rights: u32,
    recursive: bool,
    inherit: bool,
) -> Result<(), String> {
    let before = acl_entries(path).map_err(|e| format!("读取写入前 DACL 失败：{}", e))?;
    grant_one(sid, path, rights, recursive, inherit)?;
    let after = acl_entries(path).map_err(|e| format!("写后读回 DACL 失败：{}", e))?;
    let lost = lost_entries(&before, &after);
    if !lost.is_empty() {
        return Err(format!(
            "写后核对发现原有权限项丢失（{}，缺 {} 项）",
            path.display(),
            lost.len()
        ));
    }
    if !has_ace_for(sid, path, rights) {
        return Err(format!(
            "写后校验失败（{}）：给定权限的 ACE 未生效，本机环境不允许",
            path.display()
        ));
    }
    Ok(())
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

/// 目的：列出对象上显式（非继承）的包 SID 允许 ACE，供台账外孤儿授权的回收。
/// 返回：SID 字符串，形如 S-1-15-2-*；排除 ALL APPLICATION PACKAGES 与 ALL RESTRICTED 两个基线。
/// 错误：读不到对象 DACL 时返回原因。
pub(crate) fn orphan_package_aces(path: &Path) -> Result<Vec<String>, String> {
    const INHERITED_ACE: u8 = 0x10;
    Ok(acl_scan(path)?
        .into_iter()
        .filter(|(ace_type, flags, sid, _)| {
            *ace_type == 0
                && flags & INHERITED_ACE == 0
                && sid.starts_with("S-1-15-2-")
                && sid != "S-1-15-2-1"
                && sid != "S-1-15-2-2"
        })
        .map(|(_, _, sid, _)| sid)
        .collect())
}

/// 该对象上有没有给这个 SID 的**任何**允许 ACE（不看权限位）。
/// 用途：残留检查——撤权后哪怕只留一位（真机残留过一条只有 SYNCHRONIZE 的 (OI)(CI) ACE，
/// 整棵子树因此对受限进程不可读）也算没撤干净；
/// has_ace_for 回答"够不够用"，这条回答"在不在场"。
pub(crate) fn has_any_ace_for(sid: PSID, path: &Path) -> bool {
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
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
    if rc != 0 || dacl.is_null() {
        return false;
    }
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            ACL_SIZE_INFORMATION_CLASS,
        )
    };
    let mut found = false;
    if ok != 0 {
        for i in 0..info.AceCount {
            let mut ace: *mut c_void = std::ptr::null_mut();
            if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
                continue;
            }
            let base = ace as *const u8;
            if unsafe { *base } != ACCESS_ALLOWED_ACE_TYPE {
                continue;
            }
            const INHERIT_ONLY_ACE: u8 = 0x08;
            if unsafe { *base.add(1) } & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            if unsafe { EqualSid(base.add(8) as PSID, sid) } != 0 {
                found = true;
                break;
            }
        }
    }
    unsafe {
        LocalFree(sd);
    }
    found
}

/// 该对象上是不是已经有给这个 SID 的允许 ACE，**且权限位覆盖得住**。
/// 用途：基线授权只以递归方式写过一次，所以根上已有"够用"的 ACE 就跳过整棵树——否则每来一个 agent 都要重走几万文件。
/// 只看"有没有该 SID 的 ACE"不够：解释器目录会继承只有 SYNCHRONIZE 的 ALL APPLICATION PACKAGES ACE，
/// 基线因此被整条跳过，容器里连解释器都读不到（工具报 python is not recognized）。
pub(crate) fn has_ace_for(sid: PSID, path: &Path, rights: u32) -> bool {
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
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
    if rc != 0 || dacl.is_null() {
        return false;
    }
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            ACL_SIZE_INFORMATION_CLASS,
        )
    };
    let mut found = false;
    if ok != 0 {
        for i in 0..info.AceCount {
            let mut ace: *mut c_void = std::ptr::null_mut();
            if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
                continue;
            }
            let base = ace as *const u8;
            // ACCESS_ALLOWED_ACE：AceType(1) + AceFlags(1) + AceSize(2) + Mask(4) → SID 从第 8 字节开始。
            if unsafe { *base } != ACCESS_ALLOWED_ACE_TYPE {
                continue;
            }
            // 只继承给子项的 ACE 不作用于本对象，不算数。
            const INHERIT_ONLY_ACE: u8 = 0x08;
            if unsafe { *base.add(1) } & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            let mask = unsafe { std::ptr::read_unaligned(base.add(4) as *const u32) };
            if !rights_covered(mask, rights) {
                continue;
            }
            if unsafe { EqualSid(base.add(8) as PSID, sid) } != 0 {
                found = true;
                break;
            }
        }
    }
    unsafe {
        LocalFree(sd);
    }
    found
}
