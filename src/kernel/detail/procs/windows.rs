//! 目的：`procs` 的 Windows 后端——作业对象、进程创建时间与 `taskkill`。
//! 管：父死子亡的绑定、身份判据、存活探测与连树终止、对账的身份结论。
//! 不管：账本读写与对账流程（在门面）；命令行标记（Unix 的判据）。
//! 联动：由 `src/kernel/detail/procs.rs` 按平台选中。

use super::*;
use std::process::Command;

/// 目的：Windows 不需要在 spawn 前配置进程树（作业对象在 spawn 后绑定）。
pub(super) fn guard(_cmd: &mut Command) {}

/// 目的：Windows 直接起进程（作业对象与发起线程无关）。
pub(super) fn spawn(mut cmd: Command) -> std::io::Result<Child> {
    cmd.spawn()
}

/// 目的：把刚起的子进程绑到本进程的作业对象上（核心一死，它们跟着死）。
pub(super) fn bind(child: &Child) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    assign(child.as_raw_handle())
}

/// 目的：本进程唯一的作业对象（0 = 建不起来；只建一次，核心一死句柄一关，里面的进程跟着死）。
fn job_object() -> usize {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    *JOB.get_or_init(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if h.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return 0;
        }
        h as usize
    })
}

/// 目的：把一个进程句柄绑进作业对象；系统按句柄的访问权决定成败。
/// 约束：拆出来是为了让失败分支可注入——传一个只带查询权的句柄就能造出真实失败。
pub(crate) fn assign(process: *mut std::ffi::c_void) -> Result<(), String> {
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    let job = job_object();
    if job == 0 {
        return Err("建作业对象失败（子进程不会被作业兜底）".to_string());
    }
    if unsafe { AssignProcessToJobObject(job as *mut _, process) } == 0 {
        return Err("把子进程绑进作业对象失败".to_string());
    }
    Ok(())
}

/// 目的：进程创建时间（FILETIME）——对账时用它区分「还是我们那一个」与「pid 被复用」。
pub(super) fn created(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut birth: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(h, &mut birth, &mut exit, &mut kernel, &mut user);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(((birth.dwHighDateTime as u64) << 32) | birth.dwLowDateTime as u64)
    }
}

/// 目的：Windows 由作业对象兜底，不需要额外的守门进程。
pub(super) fn watch(_child: &Child) -> Result<(), String> {
    Ok(())
}

/// 目的：Windows 没有守门进程模式；真被调起时如实报错。
pub(super) fn watchdog_serve(_parent: u32, _child: u32) -> i32 {
    eprintln!("[进程] 守门进程只在 macOS 上存在：Windows 走作业对象");
    2
}

/// 目的：用 `OpenProcess` + `GetExitCodeProcess` 判活。
pub(super) fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

/// 目的：用 `taskkill /T /F` 连整棵树收掉。
pub(super) fn terminate(pid: u32) -> Result<(), String> {
    let out = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .map_err(|e| format!("taskkill 起不来：{}", e))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "taskkill 失败：{}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// 目的：对账的身份结论——创建时间对得上才是我们的；账上没有创建时间就只报不动手。
pub(super) fn ownership(entry: &Entry) -> Ownership {
    match (entry.created, created(entry.pid)) {
        (Some(expected), Some(actual)) if expected == actual => Ownership::Ours,
        (Some(_), Some(_)) => Ownership::Foreign,
        (Some(_), None) => Ownership::Gone,
        (None, _) => Ownership::Unknown,
    }
}
