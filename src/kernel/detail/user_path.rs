//! 目的：用户级 PATH 的真实实现——读写 HKCU\Environment 的 `Path`，并广播 WM_SETTINGCHANGE。
//! 管：注册表类型（REG_SZ / REG_EXPAND_SZ）的原样保持、UTF-16 编解码与一次尽力广播。
//! 不管：PATH 该不该改、怎么分段去重（在 entry::setup::path 的纯函数里）；测试不碰真实 HKCU（替身实现 UserPath）。
//! 联动：entry::setup::path 只依赖端口；本文件是端口在 Windows 上的唯一实现。

#![cfg(windows)]

use crate::kernel::ports::{PathValue, UserPath};
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

/// 目的：环境变量所在的键与值名（用户级，不改系统级 PATH）。
const ENV_KEY: &str = "Environment";
const PATH_VALUE: &str = "Path";

/// 目的：HKCU\Environment 的实现（不需要管理员）。
pub struct WindowsUserPath;

impl WindowsUserPath {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsUserPath {
    fn default() -> Self {
        Self::new()
    }
}

/// 目的：Rust 字符串 → 以 NUL 结尾的 UTF-16（Win32 的 LPCWSTR）。
fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 目的：裸缓冲区 → Rust 字符串；注册表里可能多的尾部 NUL 一并去掉。
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

impl UserPath for WindowsUserPath {
    fn read(&self) -> Result<PathValue, String> {
        let key = wide(ENV_KEY);
        let mut hkey = std::ptr::null_mut();
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        };
        if rc != 0 {
            return Err(format!("打开 HKCU\\Environment 失败（错误码 {rc}）"));
        }
        let name = wide(PATH_VALUE);
        let mut ty: u32 = 0;
        let mut size: u32 = 0;
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null(),
                &mut ty,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if rc != 0 {
            unsafe { RegCloseKey(hkey) };
            // 值不存在 = 空 PATH（调用方会在末尾追加我们那段）。
            if rc == 2 {
                return Ok(PathValue {
                    text: String::new(),
                    expand: false,
                });
            }
            return Err(format!("读用户 PATH 长度失败（错误码 {rc}）"));
        }
        let mut buf = vec![0u16; (size as usize / 2).max(1)];
        let mut got = size;
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null(),
                &mut ty,
                buf.as_mut_ptr() as *mut u8,
                &mut got,
            )
        };
        unsafe { RegCloseKey(hkey) };
        if rc != 0 {
            return Err(format!("读用户 PATH 失败（错误码 {rc}）"));
        }
        let expand = match ty {
            REG_EXPAND_SZ => true,
            REG_SZ => false,
            other => {
                return Err(format!(
                    "用户 PATH 的类型不是 REG_SZ/REG_EXPAND_SZ（收到 {other}），不敢改写"
                ));
            }
        };
        buf.truncate(got as usize / 2);
        Ok(PathValue {
            text: from_wide(&buf),
            expand,
        })
    }

    fn write(&self, value: &PathValue) -> Result<(), String> {
        let key = wide(ENV_KEY);
        let mut hkey = std::ptr::null_mut();
        let rc =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_SET_VALUE, &mut hkey) };
        if rc != 0 {
            return Err(format!("打开 HKCU\\Environment 失败（错误码 {rc}）"));
        }
        let name = wide(PATH_VALUE);
        let data = wide(&value.text);
        let ty = if value.expand { REG_EXPAND_SZ } else { REG_SZ };
        let rc = unsafe {
            RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                ty,
                data.as_ptr() as *const u8,
                (data.len() * 2) as u32,
            )
        };
        unsafe { RegCloseKey(hkey) };
        if rc != 0 {
            return Err(format!("写用户 PATH 失败（错误码 {rc}）"));
        }
        Ok(())
    }

    fn broadcast(&self) -> Result<(), String> {
        // 目的：让资源管理器与之后新起的进程重新读环境块；没有窗口回应不算失败（改变量本身已生效）。
        let param = wide("Environment");
        let mut result: usize = 0;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST as HWND,
                WM_SETTINGCHANGE,
                0 as WPARAM,
                param.as_ptr() as LPARAM,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            );
        }
        Ok(())
    }
}
