//! 目的：SID 与文本互转——台账里存 SID 文本，改 ACL 要 SID 指针。
//! 管：SID → 文本、文本 → SID 两条转换及各自的错误文案。
//! 不管：SID 从哪来（container.rs 派生容器与能力 SID）、ACL 怎么改（acl.rs）。
//! 联动：journal.rs 用文本做台账键；acl.rs 与 confine/prep.rs 用 sid_to_string 打授权足迹。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows_sys::Win32::Security::PSID;

/// 目的：SID → 字符串（写台账用）。
pub(crate) fn sid_to_string(sid: PSID) -> String {
    let mut out: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 || out.is_null() {
        return "(未知 SID)".to_string();
    }
    let mut buf: Vec<u16> = Vec::new();
    unsafe {
        let mut i = 0isize;
        loop {
            let c = *out.offset(i);
            if c == 0 {
                break;
            }
            buf.push(c);
            i += 1;
        }
        LocalFree(out as *mut c_void);
    }
    String::from_utf16_lossy(&buf)
}

/// 目的：字符串 → SID（清理时按台账里的字符串还原）。
pub(crate) fn sid_from_string(text: &str) -> Result<PSID, String> {
    let w: Vec<u16> = std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err(format!("SID 不合法：{}", text));
    }
    Ok(sid)
}
