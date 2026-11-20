//! 目的：ACL 纯逻辑的脱机单测——ACE 布局解析、身份判等、继承位、通用位展开与覆盖关系、完整性标签判据。
//! 管：ace_parts / entry_of / is_object_ace / inheritable_flags_ok / expand_generics / rights_covered / parse_label_sacl / blocking_integrity_rid 的用例。
//! 不管：需要真机 DACL 的路径（tests/<平台>/ 的真机探针覆盖）。
//! 联动：acl.rs 汇总导出；用例只走纯字节，不碰 IO。

use super::ace::*;
use super::label::*;
use super::verify::inheritable_flags_ok;
use windows_sys::Win32::Security::{
    ACE_INHERITED_OBJECT_TYPE_PRESENT, ACE_OBJECT_TYPE_PRESENT, CONTAINER_INHERIT_ACE,
    OBJECT_INHERIT_ACE,
};

/// 目的：继承位判据必须同时带 (OI) 与 (CI)：只带一条的旧 ACE 不构成「根下既有与新建子项都继承得到」。
#[test]
fn inheritable_flags_require_both_directions() {
    let both = (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8;
    assert!(inheritable_flags_ok(both));
    assert!(!inheritable_flags_ok(OBJECT_INHERIT_ACE as u8));
    assert!(!inheritable_flags_ok(CONTAINER_INHERIT_ACE as u8));
    assert!(!inheritable_flags_ok(both | INHERIT_ONLY_ACE));
}

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

/// 标准允许 / 拒绝 ACE：SID 紧跟在掩码之后（偏移 8）。
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

/// 造一条强制标签 ACE：S-1-16-<rid> + 策略掩码。
fn label_ace(rid: u32, policy: u32) -> Vec<u8> {
    let mut sid = vec![1u8, 1, 0, 0, 0, 0, 0, 16];
    sid.extend_from_slice(&rid.to_le_bytes());
    ace_bytes(SYSTEM_MANDATORY_LABEL_ACE_TYPE, 0, policy, 0, 0, &sid)
}

/// 造一份 SACL：头（修订 2、总长、条数）+ 逐条 ACE。
fn sacl_of(aces: &[Vec<u8>]) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    for a in aces {
        body.extend_from_slice(a);
    }
    let size = (8 + body.len()) as u16;
    let mut out = vec![2u8, 0];
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(aces.len() as u16).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&body);
    out
}

/// 目的：无标签是容器能写的正常基线——无 SACL、空 SACL 都不算命中。
#[test]
fn unlabeled_object_is_the_writable_baseline() {
    assert_eq!(blocking_integrity_rid(&UNLABELED), None);
    assert_eq!(parse_label_sacl(&[]).expect("空切片当作无 SACL").rid, None);
    assert_eq!(
        parse_label_sacl(&sacl_of(&[]))
            .expect("空 SACL 认得出来")
            .rid,
        None
    );
}

/// 目的：与容器同级（Low）不算向上写，不挡。
#[test]
fn low_label_does_not_block_the_container() {
    let label = parse_label_sacl(&sacl_of(&[label_ace(0x1000, 1)])).expect("Low 标签认得出来");
    assert_eq!(label.rid, Some(0x1000));
    assert_eq!(blocking_integrity_rid(&label), None, "同级不算向上写");
}

/// 目的：高于容器的级别带「不许向上写」就挡——Medium / High / System 逐条钉住。
#[test]
fn higher_labels_with_no_write_up_block_the_container() {
    for rid in [0x2000u32, 0x3000, 0x4000] {
        let label = parse_label_sacl(&sacl_of(&[label_ace(rid, 1)])).expect("标签认得出来");
        assert_eq!(label.rid, Some(rid));
        assert_eq!(label.policy, MANDATORY_POLICY_NO_WRITE_UP);
        assert_eq!(
            blocking_integrity_rid(&label),
            Some(rid),
            "RID {:#x} 应当挡写",
            rid
        );
    }
}

/// 目的：策略里没有「不许向上写」就不挡写（只读向上限制不在本判据范围内）。
#[test]
fn label_without_no_write_up_does_not_block_writes() {
    for policy in [0u32, 0x2] {
        let label = parse_label_sacl(&sacl_of(&[label_ace(0x2000, policy)])).expect("标签认得出来");
        assert_eq!(
            blocking_integrity_rid(&label),
            None,
            "策略 {:#x} 不含不许向上写",
            policy
        );
    }
}

/// 目的：标签 ACE 不一定排在第一条——按 AceSize 逐条走，前面的审计 ACE 要跳得过去。
#[test]
fn label_ace_is_found_after_other_sacl_entries() {
    let audit = ace_bytes(2, 0, 0, 0, 0, &sid_tail());
    let label = parse_label_sacl(&sacl_of(&[audit, label_ace(0x2000, 1)])).expect("标签认得出来");
    assert_eq!(label.rid, Some(0x2000));
    assert_eq!(blocking_integrity_rid(&label), Some(0x2000));
}

/// 目的：布局认不出就如实报错，绝不猜成「无标签」——猜错等于把挡写的事实静默吞掉。
#[test]
fn malformed_sacl_is_reported_not_guessed_as_unlabeled() {
    assert!(parse_label_sacl(&[2, 0, 8]).is_err(), "头长度不够要报");
    let full = sacl_of(&[label_ace(0x2000, 1)]);
    let mut zero_size = full.clone();
    zero_size[10] = 0;
    zero_size[11] = 0;
    assert!(parse_label_sacl(&zero_size).is_err(), "AceSize 为 0 要报");
    assert!(
        parse_label_sacl(&full[..full.len() - 4]).is_err(),
        "ACE 越界要报"
    );
    let mut no_subs = full.clone();
    no_subs[17] = 0;
    assert!(parse_label_sacl(&no_subs).is_err(), "子权威数为 0 要报");
    // 标识机构不是 S-1-16-：这不是完整性标签，认不出就报。
    let mut wrong_authority = full.clone();
    wrong_authority[23] = 5;
    assert!(parse_label_sacl(&wrong_authority).is_err());
}

/// 目的：诊断文案的级别名——已知级别给名字，其余给 RID 文本。
#[test]
fn integrity_names_cover_the_known_levels() {
    assert_eq!(integrity_name(0x0000), "Untrusted");
    assert_eq!(integrity_name(0x1000), "Low");
    assert_eq!(integrity_name(0x2000), "Medium");
    assert_eq!(integrity_name(0x3000), "High");
    assert_eq!(integrity_name(0x4000), "System");
    assert_eq!(integrity_name(0x5000), "RID 20480");
}
