//! 目的：对象的强制完整性标签——读出级别与策略，判定它会不会挡住容器（Low 完整性）写入。
//! 管：LABEL_SECURITY_INFORMATION 的读数、SACL 里 SYSTEM_MANDATORY_LABEL_ACE 的解析、blocking_integrity_rid 判据。
//! 不管：DACL 的读写与核对（scan.rs / grant.rs / verify.rs）、命中后拒不拒绝执行（windows.rs 的 prepare_fence）。
//! 联动：判据由 prepare_fence 在授权后逐落点调用；ACE 与 SID 的字节口径与 ace.rs 同源。

use super::*;
use std::path::Path;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::{ACL, PSID};

/// 目的：读安全描述符的强制标签部分（SECURITY_INFORMATION 的 LABEL_SECURITY_INFORMATION）。
pub(crate) const LABEL_SECURITY_INFORMATION: u32 = 0x0000_0010;
/// 目的：强制标签 ACE 的类型（SDK 的 SYSTEM_MANDATORY_LABEL_ACE_TYPE）。
pub(crate) const SYSTEM_MANDATORY_LABEL_ACE_TYPE: u8 = 0x11;
/// 目的：标签策略「不许向上写」（SYSTEM_MANDATORY_POLICY_NO_WRITE_UP）。
pub(crate) const MANDATORY_POLICY_NO_WRITE_UP: u32 = 0x0000_0001;
/// 目的：容器令牌的完整性级别——AppContainer 恒为 Low（SECURITY_MANDATORY_LOW_RID）。
pub(crate) const CONTAINER_INTEGRITY_RID: u32 = 0x0000_1000;
/// 目的：强制标签 SID 的标识机构（SECURITY_MANDATORY_LABEL_AUTHORITY，S-1-16-…）。
const MANDATORY_LABEL_AUTHORITY: u8 = 16;
/// 目的：ACL 头的长度——AclRevision(1) + Sbz1(1) + AclSize(2) + AceCount(2) + Sbz2(2)。
const ACL_HEADER_LEN: usize = 8;
/// 目的：标签 ACE 里 SID 的起始偏移——AceType(1) + AceFlags(1) + AceSize(2) + Mask(4)。
const LABEL_SID_OFFSET: usize = 8;
/// 目的：SID 头的长度——Revision(1) + SubAuthorityCount(1) + IdentifierAuthority(6)。
const SID_HEADER_LEN: usize = 8;

/// 目的：一个对象的强制完整性标签读数。
/// 约束：纯数据、不持有 IO 资源，可脱机单测。
pub(crate) struct IntegrityLabel {
    /// 目的：标签级别 RID（S-1-16-<rid> 的末位子权威）；None = 这个对象没有标签。
    pub(crate) rid: Option<u32>,
    /// 目的：标签策略掩码；无标签时为 0。
    pub(crate) policy: u32,
}

/// 目的：无标签的读数（无 SACL、空 SACL、或 SACL 里没有标签 ACE）。
pub(crate) const UNLABELED: IntegrityLabel = IntegrityLabel {
    rid: None,
    policy: 0,
};

/// 目的：这个标签挡住容器写入时给出它的级别 RID。
/// 返回：标签级别高于容器、且策略带「不许向上写」时给 Some(rid)，否则 None。
/// 约束：**无标签不算命中**——未打标签的对象不做完整性检查，那正是容器能写的正常基线。
pub(crate) fn blocking_integrity_rid(label: &IntegrityLabel) -> Option<u32> {
    let rid = label.rid?;
    (rid > CONTAINER_INTEGRITY_RID && label.policy & MANDATORY_POLICY_NO_WRITE_UP != 0)
        .then_some(rid)
}

/// 目的：给人看的标签级别名（诊断文案用）。
/// 返回：已知级别给名字，其余给 RID 的十进制文本。
pub(crate) fn integrity_name(rid: u32) -> String {
    match rid {
        0x0000 => "Untrusted".to_string(),
        CONTAINER_INTEGRITY_RID => "Low".to_string(),
        0x2000 => "Medium".to_string(),
        0x3000 => "High".to_string(),
        0x4000 => "System".to_string(),
        other => format!("RID {}", other),
    }
}

/// 目的：从 SACL 字节里认出强制标签（纯解析，不读盘）。
/// 参数：bytes 是 SACL 的原始字节；空切片当作无 SACL。
/// 返回：第一条标签 ACE 的级别与策略；没有标签 ACE 时给 UNLABELED。
/// 错误：布局认不出（头长度不够、AceSize 为 0 或越界、标签 SID 不是 S-1-16-）时如实报错。
/// 约束：宁可报「认不出」也不猜——猜成无标签就等于把挡写的事实静默吞掉。
pub(crate) fn parse_label_sacl(bytes: &[u8]) -> Result<IntegrityLabel, String> {
    if bytes.is_empty() {
        return Ok(UNLABELED);
    }
    if bytes.len() < ACL_HEADER_LEN {
        return Err(format!("SACL 头长度不够（{} 字节）", bytes.len()));
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut at = ACL_HEADER_LEN;
    for i in 0..count {
        if at + LABEL_SID_OFFSET > bytes.len() {
            return Err(format!("SACL 第 {} 条 ACE 越界（偏移 {}）", i + 1, at));
        }
        let ace_type = bytes[at];
        let size = u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if size < LABEL_SID_OFFSET || at + size > bytes.len() {
            return Err(format!("SACL 的 ACE 长度不合理（{}，偏移 {}）", size, at));
        }
        if ace_type == SYSTEM_MANDATORY_LABEL_ACE_TYPE {
            let policy =
                u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]);
            let rid = label_sid_rid(&bytes[at + LABEL_SID_OFFSET..at + size])?;
            return Ok(IntegrityLabel {
                rid: Some(rid),
                policy,
            });
        }
        at += size;
    }
    Ok(UNLABELED)
}

/// 目的：从标签 ACE 的 SID 字节里取级别 RID。
/// 参数：bytes 是 SID 起始到 ACE 末尾的切片。
/// 返回：末位子权威（S-1-16-<rid> 的 rid）。
/// 错误：SID 头不够、子权威数为 0、或标识机构不是 16 时报错。
fn label_sid_rid(bytes: &[u8]) -> Result<u32, String> {
    if bytes.len() < SID_HEADER_LEN {
        return Err(format!("标签 SID 头长度不够（{} 字节）", bytes.len()));
    }
    let subs = bytes[1] as usize;
    if subs == 0 || bytes.len() < SID_HEADER_LEN + subs * 4 {
        return Err(format!("标签 SID 子权威数与长度不符（{} 个）", subs));
    }
    // 标识机构是 6 字节大端：S-1-16- 要求前 5 字节为 0、末字节为 16。
    if bytes[2..7] != [0; 5] || bytes[7] != MANDATORY_LABEL_AUTHORITY {
        return Err("标签 SID 的标识机构不是 S-1-16-".to_string());
    }
    let last = SID_HEADER_LEN + (subs - 1) * 4;
    Ok(u32::from_le_bytes([
        bytes[last],
        bytes[last + 1],
        bytes[last + 2],
        bytes[last + 3],
    ]))
}

/// 目的：读一个对象的强制完整性标签（只读；读标签不需要提权，也不需要 SeSecurityPrivilege）。
/// 参数：path 是要检查的落点。
/// 返回：无 SACL 或 SACL 里没有标签 ACE 时给 UNLABELED。
/// 错误：读不到安全描述符、或 SACL 布局认不出时如实返回。
pub(crate) fn integrity_label(path: &Path) -> Result<IntegrityLabel, String> {
    let mut sacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sacl,
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!(
            "读完整性标签失败（{}）：错误码 {}",
            path.display(),
            rc
        ));
    }
    let parsed = if sacl.is_null() {
        Ok(UNLABELED)
    } else {
        let size = unsafe {
            u16::from_le_bytes([*(sacl as *const u8).add(2), *(sacl as *const u8).add(3)])
        } as usize;
        let bytes = unsafe { std::slice::from_raw_parts(sacl as *const u8, size) };
        parse_label_sacl(bytes)
    };
    unsafe {
        LocalFree(sd);
    }
    parsed
}
