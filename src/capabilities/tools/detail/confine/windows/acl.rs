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
    EXPLICIT_ACCESS_W, GRANT_ACCESS, OBJECTS_AND_SID, REVOKE_ACCESS, TRUSTEE_IS_OBJECTS_AND_SID,
    TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    GetAce, GetAclInformation, GetSecurityDescriptorLength, SetFileSecurityW,
    ACE_INHERITED_OBJECT_TYPE_PRESENT, ACE_OBJECT_TYPE_PRESENT, ACL, ACL_SIZE_INFORMATION,
    CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE, PSECURITY_DESCRIPTOR,
    PSID, SID,
};

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
use super::*;

/// ACE 类型：允许 / 拒绝各三种形态。windows-sys 不导出这些常量，按 SDK 的 ACE 头自己认。
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 5;
const ACCESS_DENIED_OBJECT_ACE_TYPE: u8 = 6;
const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 7;
const ACCESS_DENIED_CALLBACK_ACE_TYPE: u8 = 8;
const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 9;
const ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE: u8 = 10;

/// ACE 头 + 掩码：AceType(1) + AceFlags(1) + AceSize(2) + Mask(4)。
const ACE_FIXED_LEN: usize = 8;
/// 对象 ACE 在掩码之后还有 4 字节 Flags，说明后面带不带那两个 GUID。
const ACE_OBJECT_FLAGS_LEN: usize = 4;
/// 一个 GUID 的字节数。
const GUID_LEN: usize = 16;
/// 目的：ACE 标志位「只继承给子项」——带这一位的 ACE 不作用于本对象，判定时要排除。
pub(crate) const INHERIT_ONLY_ACE: u8 = 0x08;
/// 目的：ACE 标志位「由父目录继承而来」——用来区分显式写下的 ACE 与继承来的。
pub(crate) const INHERITED_ACE: u8 = 0x10;

/// 目的：一条 ACE 解析出来的关键字段——**SID 偏移随 ACE 类型变**，所以它必须算出来，不能到处硬写 8。
/// 约束：纯数据（GUID 已转文本），不持有 IO 资源，可脱机单测。
pub(crate) struct AceParts {
    pub(crate) ace_type: u8,
    pub(crate) flags: u8,
    pub(crate) mask: u32,
    /// 目的：本条 ACE 里 SID 起始的字节偏移——随 ACE 类型变，所以读它的地方不许硬写 8。
    pub(crate) sid_offset: usize,
    /// 目的：对象 ACE 的类型 GUID（十六进制文本）；非对象 ACE 是空串。
    pub(crate) object_type: String,
    /// 目的：对象 ACE 的继承对象类型 GUID；非对象 ACE 是空串。
    pub(crate) inherited_object_type: String,
}

/// 目的：一条 ACE 的读数结果（原始字段 + SID 文本），供比对与诊断转储共用。
#[derive(Clone)]
pub(crate) struct AceView {
    pub(crate) ace_type: u8,
    pub(crate) flags: u8,
    pub(crate) mask: u32,
    pub(crate) sid: String,
    pub(crate) object_type: String,
    pub(crate) inherited_object_type: String,
}

/// 目的：一个对象 DACL 的读数——权限项逐条 + 认不出来的条数（不静默吞掉异常 ACE）。
pub(crate) struct DaclScan {
    pub(crate) entries: Vec<AceView>,
    /// 目的：进不了比对、也不该被吞掉的条数——类型不在已知布局内，或字节不够一条完整 ACE。
    pub(crate) unparsed: usize,
}

/// 目的：权限项在比对里的**身份**——（ACE 类型, SID, 权限位, 对象类型 GUID, 继承对象类型 GUID）。
/// 约束：GUID 必须在身份里：只差 GUID 的两条对象 ACE 不是同一条，否则弄丢一条会被另一条顶包。
pub(crate) type AceEntry = (u8, String, u32, String, String);

/// 目的：这条 ACE 是不是**权限项**（允许 / 拒绝，含对象与回调形态）——写后核对比的就是它们。
/// 约束：系统审计（2/3/11/12）、强制标签（13）、资源属性（16）、访问过滤（17）不是权限项，不参与比对。
pub(crate) fn is_permission_ace(ace_type: u8) -> bool {
    matches!(
        ace_type,
        ACCESS_ALLOWED_ACE_TYPE
            | ACCESS_DENIED_ACE_TYPE
            | ACCESS_ALLOWED_OBJECT_ACE_TYPE
            | ACCESS_DENIED_OBJECT_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE
    )
}

/// 目的：这条 ACE 的 SID 之前是不是还夹着 Flags(4) 与 0/1/2 个 GUID（对象 ACE 与回调对象 ACE）。
/// 约束：只有 5 / 6 / 9 / 10 是这种布局；7 / 8 这类回调 ACE 的额外数据在 **SID 之后**，偏移仍从 8 起。
fn is_object_ace(ace_type: u8) -> bool {
    matches!(
        ace_type,
        ACCESS_ALLOWED_OBJECT_ACE_TYPE
            | ACCESS_DENIED_OBJECT_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE
    )
}

/// 目的：这条 ACE 的 SID 是不是紧跟在掩码之后（标准 ACE 与"额外数据在 SID 之后"的那些类型）。
/// 约束：只认我们确认过布局的类型；别的（含已废弃的 4 号复合 ACE）返回 false，宁可报"认不出"也不猜。
fn sid_follows_mask(ace_type: u8) -> bool {
    matches!(
        ace_type,
        ACCESS_ALLOWED_ACE_TYPE
            | ACCESS_DENIED_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_ACE_TYPE
            | 2
            | 3
            | 11
            | 12
            | 13
            | 14
            | 16
            | 17
    )
}

/// 目的：从一条 ACE 的原始字节里认出类型 / 标志 / 掩码 / SID 偏移 / 两个对象 GUID——偏移的**唯一**算法来源。
/// 参数：bytes 是单条 ACE 的完整字节（GetAce 给的指针按 AceSize 切出来的那一段）。
/// 返回：布局认得出来给 AceParts；类型不认识、或缺字节时给 None（调用方如实计入 DaclScan::unparsed）。
/// 约束：只读传进来的字节，不 deref 字节之外的东西——纯逻辑，脱机可测。
pub(crate) fn ace_parts(bytes: &[u8]) -> Option<AceParts> {
    if bytes.len() < ACE_FIXED_LEN {
        return None;
    }
    let ace_type = bytes[0];
    let mask = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let mut sid_offset = ACE_FIXED_LEN;
    let mut object_type = String::new();
    let mut inherited_object_type = String::new();
    if is_object_ace(ace_type) {
        if bytes.len() < ACE_FIXED_LEN + ACE_OBJECT_FLAGS_LEN {
            return None;
        }
        let object_flags = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        sid_offset += ACE_OBJECT_FLAGS_LEN;
        if object_flags & ACE_OBJECT_TYPE_PRESENT != 0 {
            object_type = guid_text(bytes.get(sid_offset..sid_offset + GUID_LEN)?);
            sid_offset += GUID_LEN;
        }
        if object_flags & ACE_INHERITED_OBJECT_TYPE_PRESENT != 0 {
            inherited_object_type = guid_text(bytes.get(sid_offset..sid_offset + GUID_LEN)?);
            sid_offset += GUID_LEN;
        }
    } else if !sid_follows_mask(ace_type) {
        return None;
    }
    // SID 至少要有一个字节落在本条 ACE 之内；否则这条 ACE 是坏的，不按偏移硬读（防越界）。
    if sid_offset >= bytes.len() {
        return None;
    }
    Some(AceParts {
        ace_type,
        flags: bytes[1],
        mask,
        sid_offset,
        object_type,
        inherited_object_type,
    })
}

/// 目的：把 GUID 的 16 字节写成十六进制文本——GUID 只当**身份**用（比对两条对象 ACE 是不是同一条）。
/// 约束：按本机字节序逐字节转储（同一台机器上同一字节序列 → 同一文本），不按 GUID 字段序重组。
fn guid_text(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

/// 目的：把 16 字节还原成 GUID——与 guid_text 的字节序口径一致（同一段字节进出同一个值）。
/// 约束：只在写对象 ACE 时用得到；纯字节搬运，不做字段序转换。
fn guid_from_bytes(bytes: [u8; GUID_LEN]) -> windows_sys::core::GUID {
    windows_sys::core::GUID {
        data1: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        data2: u16::from_le_bytes([bytes[4], bytes[5]]),
        data3: u16::from_le_bytes([bytes[6], bytes[7]]),
        data4: [
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ],
    }
}

/// 目的：把一条读数压成比对身份——读 DACL 与纯逻辑单测共用同一处口径。
pub(crate) fn entry_of(view: &AceView) -> AceEntry {
    (
        view.ace_type,
        view.sid.clone(),
        view.mask,
        view.object_type.clone(),
        view.inherited_object_type.clone(),
    )
}
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
/// 所以读回来时必须用 ace_parts 算出的偏移解析（这正是本模块两处口径要对齐的地方）。
fn write_ace(
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
fn permission_entries(scan: &DaclScan) -> Vec<AceEntry> {
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
    let before = acl_scan(path).map_err(|e| format!("读取写入前 DACL 失败：{}", e))?;
    grant_one(sid, path, rights, recursive, inherit)?;
    let after = acl_scan(path).map_err(|e| format!("写后读回 DACL 失败：{}", e))?;
    let lost = lost_entries(&permission_entries(&before), &permission_entries(&after));
    if !lost.is_empty() {
        // 认不出的 ACE 进不了比对：有就说清，别让"缺 N 项"看起来像全部账目。
        let unparsed = if after.unparsed > 0 {
            format!("；另有 {} 条认不出的 ACE 未参与比对", after.unparsed)
        } else {
            String::new()
        };
        return Err(format!(
            "写后核对发现原有权限项丢失（{}，缺 {} 项{}）",
            path.display(),
            lost.len(),
            unparsed
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
    Ok(acl_scan(path)?
        .entries
        .into_iter()
        .filter(|v| {
            v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                && v.flags & INHERITED_ACE == 0
                && v.sid.starts_with("S-1-15-2-")
                && v.sid != "S-1-15-2-1"
                && v.sid != "S-1-15-2-2"
        })
        .map(|v| v.sid)
        .collect())
}

/// 目的：这段文本是不是一个真的 SID——转换失败时 sid_to_string 会给占位文本，它不能当判等依据。
fn is_sid_text(text: &str) -> bool {
    text.starts_with("S-1-")
}

/// 该对象上有没有给这个 SID 的**任何**允许 ACE（不看权限位）。
/// 用途：残留检查——撤权后哪怕只留一位（真机残留过一条只有 SYNCHRONIZE 的 (OI)(CI) ACE，
/// 整棵子树因此对受限进程不可读）也算没撤干净；
/// has_ace_for 回答"够不够用"，这条回答"在不在场"。
/// 约束：只看标准允许 ACE（对象 ACE 是给子对象的，不属于"本对象上的残留"）。
pub(crate) fn has_any_ace_for(sid: PSID, path: &Path) -> bool {
    let want = sid_to_string(sid);
    if !is_sid_text(&want) {
        return false;
    }
    acl_scan(path)
        .map(|scan| {
            scan.entries.iter().any(|v| {
                v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    && v.flags & INHERIT_ONLY_ACE == 0
                    && v.sid == want
            })
        })
        .unwrap_or(false)
}

/// 该对象上是不是已经有给这个 SID 的允许 ACE，**且权限位覆盖得住**。
/// 用途：基线授权只以递归方式写过一次，所以根上已有"够用"的 ACE 就跳过整棵树——否则每来一个 agent 都要重走几万文件。
/// 只看"有没有该 SID 的 ACE"不够：解释器目录会继承只有 SYNCHRONIZE 的 ALL APPLICATION PACKAGES ACE，
/// 基线因此被整条跳过，容器里连解释器都读不到（工具报 python is not recognized）。
/// 约束：只看标准允许 ACE（对象 ACE 是给子对象的，不构成"本对象上已经够用"）。
pub(crate) fn has_ace_for(sid: PSID, path: &Path, rights: u32) -> bool {
    let want = sid_to_string(sid);
    if !is_sid_text(&want) {
        return false;
    }
    acl_scan(path)
        .map(|scan| {
            scan.entries.iter().any(|v| {
                v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    && v.flags & INHERIT_ONLY_ACE == 0
                    && rights_covered(v.mask, rights)
                    && v.sid == want
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一条 ACE 的原始字节：头 + 掩码 +（对象 ACE 才有）Flags 与在场 GUID + 尾部（SID 那一段）。
    /// 参数：guids 是后面接几个 GUID（0/1/2），object_flags 决定解析时"哪几个在场"。
    fn ace_bytes(
        ace_type: u8,
        flags: u8,
        mask: u32,
        object_flags: u32,
        guids: usize,
        tail: &[u8],
    ) -> Vec<u8> {
        let mut body: Vec<u8> = Vec::new();
        // 对象 ACE 总是有 Flags(4)：它说明后面跟着几个 GUID，即使一个都不带。
        if is_object_ace(ace_type) {
            body.extend_from_slice(&object_flags.to_le_bytes());
            for i in 0..guids {
                body.extend_from_slice(&[(i as u8) + 1; GUID_LEN]);
            }
        }
        body.extend_from_slice(tail);
        let size = (ACE_FIXED_LEN + body.len()) as u16;
        let mut out = vec![ace_type, flags];
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&mask.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// 一段"假 SID"尾巴：够用来判偏移，真 SID 由系统那边保证。
    fn sid_tail() -> Vec<u8> {
        vec![1, 2, 0, 0, 0, 0, 0, 5]
    }

    /// 标准允许 / 拒绝 ACE：SID 紧跟在掩码之后（偏移 8）——原来的硬编码在这一类上是对的。
    #[test]
    fn standard_ace_puts_the_sid_right_after_the_mask() {
        for ace_type in [ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE] {
            let bytes = ace_bytes(ace_type, 0x10, 0x0012_01BF, 0, 0, &sid_tail());
            let parts = ace_parts(&bytes).expect("标准 ACE 要认得出来");
            assert_eq!(parts.ace_type, ace_type);
            assert_eq!(parts.flags, 0x10);
            assert_eq!(parts.mask, 0x0012_01BF);
            assert_eq!(
                parts.sid_offset, ACE_FIXED_LEN,
                "标准 ACE 的 SID 从第 8 字节起"
            );
            assert!(parts.object_type.is_empty(), "标准 ACE 没有对象 GUID");
            assert!(parts.inherited_object_type.is_empty());
        }
    }

    /// 对象 ACE：SID 之前还有 Flags(4) 与在场 GUID——偏移必须按标志位算，硬写 8 会读到 GUID 的字节。
    #[test]
    fn object_ace_skips_the_guids_it_declares() {
        let both = ACE_OBJECT_TYPE_PRESENT | ACE_INHERITED_OBJECT_TYPE_PRESENT;
        let bytes = ace_bytes(ACCESS_ALLOWED_OBJECT_ACE_TYPE, 0, 0x1, both, 2, &sid_tail());
        let parts = ace_parts(&bytes).expect("两个 GUID 都在场的对象 ACE 要认得出来");
        assert_eq!(
            parts.sid_offset,
            ACE_FIXED_LEN + ACE_OBJECT_FLAGS_LEN + 2 * GUID_LEN
        );
        assert_eq!(
            parts.object_type.len(),
            GUID_LEN * 2,
            "类型 GUID 要转成十六进制文本"
        );
        assert_ne!(
            parts.object_type, parts.inherited_object_type,
            "两个 GUID 不同，不能混成一个"
        );

        let only_object = ace_bytes(
            ACCESS_DENIED_OBJECT_ACE_TYPE,
            0,
            0x1,
            ACE_OBJECT_TYPE_PRESENT,
            1,
            &sid_tail(),
        );
        let parts = ace_parts(&only_object).expect("只有类型 GUID 的也要认得出来");
        assert_eq!(
            parts.sid_offset,
            ACE_FIXED_LEN + ACE_OBJECT_FLAGS_LEN + GUID_LEN
        );
        assert!(parts.inherited_object_type.is_empty());

        let neither = ace_bytes(ACCESS_ALLOWED_OBJECT_ACE_TYPE, 0, 0x1, 0, 0, &sid_tail());
        let parts = ace_parts(&neither).expect("一个 GUID 都不带也要认得出来");
        assert_eq!(parts.sid_offset, ACE_FIXED_LEN + ACE_OBJECT_FLAGS_LEN);
    }

    /// 回调 ACE 的额外数据在 **SID 之后**：偏移仍从 8 起；回调对象 ACE 与对象 ACE 同构。
    #[test]
    fn callback_ace_keeps_the_offset_because_its_extra_data_follows_the_sid() {
        for ace_type in [
            ACCESS_ALLOWED_CALLBACK_ACE_TYPE,
            ACCESS_DENIED_CALLBACK_ACE_TYPE,
        ] {
            let mut tail = sid_tail();
            tail.extend_from_slice(&[0xAA, 0xBB]);
            let parts =
                ace_parts(&ace_bytes(ace_type, 0, 0x1, 0, 0, &tail)).expect("回调 ACE 要认得出来");
            assert_eq!(
                parts.sid_offset, ACE_FIXED_LEN,
                "回调数据在 SID 后面，不影响偏移"
            );
        }
        let both = ACE_OBJECT_TYPE_PRESENT | ACE_INHERITED_OBJECT_TYPE_PRESENT;
        let parts = ace_parts(&ace_bytes(
            ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE,
            0,
            0x1,
            both,
            2,
            &sid_tail(),
        ))
        .expect("回调对象 ACE 要认得出来");
        assert_eq!(
            parts.sid_offset,
            ACE_FIXED_LEN + ACE_OBJECT_FLAGS_LEN + 2 * GUID_LEN
        );
    }

    /// 认不出的类型与缺字节一律拒收：宁可报"认不出"，也不按固定偏移硬读别人的内存。
    #[test]
    fn unknown_or_truncated_ace_is_refused_instead_of_guessed() {
        assert!(
            ace_parts(&ace_bytes(4, 0, 0x1, 0, 0, &sid_tail())).is_none(),
            "已废弃的复合 ACE 布局不同"
        );
        assert!(
            ace_parts(&ace_bytes(99, 0, 0x1, 0, 0, &sid_tail())).is_none(),
            "未知类型不猜"
        );
        assert!(ace_parts(&[0, 0, 4, 0]).is_none(), "连头部都不够");
        // 对象 ACE 的 Flags 没写全（掩码后面只剩 2 字节）。
        let short_object = vec![ACCESS_ALLOWED_OBJECT_ACE_TYPE, 0, 10, 0, 1, 0, 0, 0, 1, 0];
        assert!(
            ace_parts(&short_object).is_none(),
            "GUID 区缺字节不得越界读"
        );
        // SID 起点正好等于 ACE 末尾：没有任何 SID 字节。
        assert!(
            ace_parts(&ace_bytes(ACCESS_ALLOWED_ACE_TYPE, 0, 0x1, 0, 0, &[])).is_none(),
            "没有 SID 的 ACE 是坏的"
        );
    }

    /// 参与写后核对的是允许 / 拒绝两类（含对象与回调形态）；审计、标签、资源属性不是权限项。
    #[test]
    fn permission_kinds_are_allow_and_deny_only() {
        for ace_type in [0, 1, 5, 6, 7, 8, 9, 10] {
            assert!(is_permission_ace(ace_type), "{} 是权限项", ace_type);
        }
        for ace_type in [2, 3, 4, 11, 12, 13, 14, 16, 17] {
            assert!(!is_permission_ace(ace_type), "{} 不是权限项", ace_type);
        }
    }

    /// 对象 ACE 的身份必须带上两个 GUID：只差 GUID 的两条不是同一条，否则弄丢一条会被另一条顶包。
    #[test]
    fn object_ace_identity_carries_the_guids() {
        let view = AceView {
            ace_type: ACCESS_ALLOWED_OBJECT_ACE_TYPE,
            flags: 0,
            mask: 0x1,
            sid: "S-1-5-32-545".to_string(),
            object_type: "AABB".to_string(),
            inherited_object_type: String::new(),
        };
        let other = AceView {
            object_type: "CCDD".to_string(),
            ..view.clone()
        };
        assert_ne!(entry_of(&view), entry_of(&other), "GUID 不同 = 不是同一条");
        assert_eq!(entry_of(&view), entry_of(&view.clone()), "同一条要判等");
        let same = AceView {
            inherited_object_type: "EEFF".to_string(),
            ..view.clone()
        };
        assert_ne!(
            entry_of(&view),
            entry_of(&same),
            "继承对象 GUID 也是身份的一部分"
        );
    }
}
