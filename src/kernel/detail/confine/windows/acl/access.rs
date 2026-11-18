//! 目的：当前进程对该对象有没有写 DACL 的资格——属主判断、AccessCheck 预检、模拟令牌与句柄/内存收尾。
//! 管：owner_is_current_user / can_write_dacl / current_user_sid 与 Impersonation、CloseOnDrop、LocalFreeOnDrop。
//! 不管：实际授权与撤权（grant.rs）、授权后的核对（verify.rs）。
//! 联动：prepare_fence 在授权前用 can_write_dacl 预检；预检与真实授权同一把尺子。

use super::super::*;
use super::scan::is_sid_text;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::{
    ACL, AccessCheck, DACL_SECURITY_INFORMATION, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION,
    GetTokenInformation, ImpersonateSelf, OWNER_SECURITY_INFORMATION, PRIVILEGE_SET,
    PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SecurityImpersonation, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

/// 目的：目录属主是不是当前用户（能写 DACL 的充分条件；不判显式 WRITE_DAC ACE）。
pub(crate) fn owner_is_current_user(path: &Path) -> Result<bool, String> {
    let me = current_user_sid()?;
    if !is_sid_text(&me) {
        return Ok(false);
    }
    let w = wide(path);
    let mut owner: PSID = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("错误码 {}", rc));
    }
    let owner_text = sid_to_string(owner);
    unsafe {
        LocalFree(sd as _);
    }
    Ok(is_sid_text(&owner_text) && owner_text == me)
}

/// 目的：当前进程能不能写这个对象的 DACL（READ_CONTROL | WRITE_DAC）——预检与真实授权同一把尺子。
/// 约束：AccessCheck 要模拟令牌（进程令牌会错误 1309）；SD 必须带 owner/group/DACL（只给 DACL 会错误 1338）。
pub(crate) fn can_write_dacl(path: &Path) -> Result<bool, String> {
    let _imp = Impersonation::begin()?;
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 0, &mut token) } == 0 {
        return Err(format!(
            "打开线程令牌失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    let _token = CloseOnDrop(token);
    let mut owner: PSID = std::ptr::null_mut();
    let mut group: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            &mut group,
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!(
            "读安全描述符失败（{}）：错误码 {}",
            path.display(),
            rc
        ));
    }
    let _sd = LocalFreeOnDrop(sd);
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ,
        GenericWrite: FILE_GENERIC_WRITE,
        GenericExecute: FILE_GENERIC_EXECUTE,
        GenericAll: FILE_ALL_ACCESS,
    };
    let mut privileges = vec![0u8; 1024];
    let mut len = privileges.len() as u32;
    let mut granted: u32 = 0;
    let mut status: i32 = 0;
    let ok = unsafe {
        AccessCheck(
            sd,
            token,
            READ_CONTROL | WRITE_DAC,
            &mapping,
            privileges.as_mut_ptr() as *mut PRIVILEGE_SET,
            &mut len,
            &mut granted,
            &mut status,
        )
    };
    if ok == 0 {
        return Err(format!(
            "AccessCheck 失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(status != 0 && granted & (READ_CONTROL | WRITE_DAC) == (READ_CONTROL | WRITE_DAC))
}

/// 目的：模拟当前用户拿线程令牌，离开作用域时一定还原（失败路径也不把线程留在模拟态）。
struct Impersonation;

impl Impersonation {
    fn begin() -> Result<Impersonation, String> {
        if unsafe { ImpersonateSelf(SecurityImpersonation) } == 0 {
            return Err(format!(
                "模拟当前用户失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Impersonation)
    }
}

impl Drop for Impersonation {
    fn drop(&mut self) {
        unsafe { RevertToSelf() };
    }
}

/// 目的：句柄离开作用域即关闭。
struct CloseOnDrop(HANDLE);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// 目的：安全描述符离开作用域即释放。
struct LocalFreeOnDrop(PSECURITY_DESCRIPTOR);

impl Drop for LocalFreeOnDrop {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0 as _) };
    }
}

/// 目的：当前进程令牌里的用户 SID。
fn current_user_sid() -> Result<String, String> {
    use windows_sys::Win32::Foundation::HANDLE;
    let mut token: HANDLE = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err("打开进程令牌失败".to_string());
    }
    let mut len = 0u32;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
    }
    if len == 0 {
        unsafe {
            CloseHandle(token);
        }
        return Err("读取令牌用户失败".to_string());
    }
    let mut buf = vec![0u8; len as usize];
    let read =
        unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut _, len, &mut len) };
    unsafe {
        CloseHandle(token);
    }
    if read == 0 {
        return Err("读取令牌用户失败".to_string());
    }
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    Ok(sid_to_string(user.User.Sid))
}
