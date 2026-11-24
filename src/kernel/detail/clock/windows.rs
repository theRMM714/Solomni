//! 目的：`clock` 的 Windows 后端——系统本地时间。
//! 管：`now`（经 `GetLocalTime` 取本机时区的民用时间）。
//! 不管：时区换算与夏令时（系统调用已经给出本地值）。
//! 联动：机制门面在 `src/kernel/detail/clock.rs`。

use windows_sys::Win32::System::SystemInformation::GetLocalTime;

pub(super) fn now() -> super::Local {
    let mut t = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    // 安全：GetLocalTime 只写这个本栈上的 SYSTEMTIME，不保存也不回读任何外来指针。
    unsafe { GetLocalTime(&mut t) };
    super::Local {
        year: i32::from(t.wYear),
        month: u32::from(t.wMonth),
        day: u32::from(t.wDay),
        hour: u32::from(t.wHour),
        minute: u32::from(t.wMinute),
        second: u32::from(t.wSecond),
        millis: u32::from(t.wMilliseconds),
    }
}
