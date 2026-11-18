//! 目的：ACE 的字节布局与身份——把一条 ACE 认成字段、类型/标志/掩码、SID 偏移与对象 GUID，以及掩码覆盖关系。
//! 管：ACE 类型常量、AceParts / AceView / DaclScan / AceEntry、ace_parts 的偏移算法、GUID 文本与还原、通用位展开与覆盖判断。
//! 不管：怎么读对象 DACL（scan.rs）、怎么把 ACE 写进去（grant.rs）、写后核对（verify.rs）。
//! 联动：scan.rs 与 verify.rs 共用这里的偏移与身份口径；acl.rs 汇总导出。

use super::super::*;
use windows_sys::Win32::Security::{ACE_INHERITED_OBJECT_TYPE_PRESENT, ACE_OBJECT_TYPE_PRESENT};

/// 目的：ACE 类型：允许 / 拒绝各三种形态。windows-sys 不导出这些常量，按 SDK 的 ACE 头自己认。
pub(super) const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
pub(super) const ACCESS_DENIED_ACE_TYPE: u8 = 1;
pub(super) const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 5;
pub(super) const ACCESS_DENIED_OBJECT_ACE_TYPE: u8 = 6;
pub(super) const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 7;
pub(super) const ACCESS_DENIED_CALLBACK_ACE_TYPE: u8 = 8;
pub(super) const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 9;
pub(super) const ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE: u8 = 10;

/// 目的：ACE 头 + 掩码的长度——AceType(1) + AceFlags(1) + AceSize(2) + Mask(4)。
pub(super) const ACE_FIXED_LEN: usize = 8;
/// 目的：对象 ACE 掩码之后那 4 字节 Flags 的长度，它说明后面带不带那两个 GUID。
pub(super) const ACE_OBJECT_FLAGS_LEN: usize = 4;
/// 目的：一个 GUID 的字节数。
pub(super) const GUID_LEN: usize = 16;
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
pub(super) fn is_object_ace(ace_type: u8) -> bool {
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
pub(super) fn guid_from_bytes(bytes: [u8; GUID_LEN]) -> windows_sys::core::GUID {
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

/// 目的：把通用位展开成具体位：ACL 里存的是哪一套，覆盖关系比较都要等价成立。
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

/// 目的：已有 ACE 的权限位是不是覆盖得住我们需要的权限位。
pub(crate) fn rights_covered(mask: u32, rights: u32) -> bool {
    expand_generics(rights) & !expand_generics(mask) == 0
}
