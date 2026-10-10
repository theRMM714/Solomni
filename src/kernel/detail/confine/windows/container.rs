//! 目的：Windows 容器围栏的启动面——AppContainer 档案与 SID、UTF-16 转换、system shell、kill-on-close job 对象。
//! 管：在容器里拉进程这一层（Windows 独有：Linux/macOS 是「先装规则再 exec」）。
//! 不管：授权与撤销（在 acl.rs / record.rs）、策略（在 confine::prepare_fence）。
//! 联动：由 windows/mod.rs 调用；失败如实返回而非假装成功。

use super::FENCE_FAILED;
use crate::kernel::api::FenceSpec;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{PSID, SECURITY_CAPABILITIES};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
use super::*;
/// 目的：容器名：**只由 agent 名决定**——一个 agent 一个 profile，跨会话复用（数量有界），
///   外层授权与守门进程因此各自能算出同一个 SID。各会话之间的路径隔离仍由那些目录上的 ACE 决定
///   （同名 agent 的多个会话共用一个容器身份，这是"数量有界"换来的取舍）。
pub(crate) fn container_name(spec: &FenceSpec) -> String {
    // FNV-1a：只为把名字收敛成定长标识（不是安全用途），避免容器名里出现中文与路径。
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in spec.agent.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{}{:016x}", PROFILE_PREFIX, h)
}

/// 目的：名字是不是本程序建过的容器 profile（Windows 把包目录名转成小写，所以按不区分大小写比）。
pub(crate) fn is_our_profile(name: &str) -> bool {
    name.get(..PROFILE_PREFIX.len())
        .map(|head| head.eq_ignore_ascii_case(PROFILE_PREFIX))
        .unwrap_or(false)
}

/// 目的：环境不允许容器围栏时的标记（探针据此区分"环境不允许"与"代码有问题"，不互相顶包）。
pub const ENV_BLOCKED_MARK: &str = "容器围栏不可用（本环境不允许";

/// 目的：本环境**建不出**容器 profile 的标记（与 ENV_BLOCKED_MARK 同一用途：环境结论要能被机器认出来，
///   不能靠猜字符串）。改不了目录 ACL 的会话、受限令牌的会话都走这一态。
pub const PROFILE_ENV_BLOCKED_MARK: &str = "拒绝访问：本环境不允许建 AppContainer profile";

/// 目的：建（或复用）容器 profile：**必须有 profile** —— 没有 profile 的派生 SID 拿不到
///   ALL APPLICATION PACKAGES 组，连系统目录里的 cmd.exe 都打不开（实测会报"找不到文件"）。
///   已存在 = 成功（同一个名字派生出的 SID 与 profile 一致，所以外层用 Derive 预授权 ACL 也有效）。
pub(crate) fn ensure_profile(name: &str) -> Result<(), String> {
    let n: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let d: Vec<u16> = std::ffi::OsStr::new("Solomni 工具围栏")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(
            n.as_ptr(),
            n.as_ptr(),
            d.as_ptr(),
            std::ptr::null(),
            0,
            &mut sid,
        )
    };
    // S_OK = 0；E_ALREADY_EXISTS（0x800700B7）= 已有同 profile，照用。
    const E_ALREADY_EXISTS: i32 = 0x8007_00B7u32 as i32;
    if hr >= 0 || hr == E_ALREADY_EXISTS {
        if !sid.is_null() {
            free_sid(sid);
        }
        return Ok(());
    }
    const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
    if hr == E_ACCESSDENIED {
        return Err(format!(
            "0x{:08x}（{}）",
            hr as u32, PROFILE_ENV_BLOCKED_MARK
        ));
    }
    Err(format!("0x{:08x}", hr as u32))
}

/// 目的：派生容器 SID（本机实测：非管理员可用）。
pub(crate) fn container_sid(name: &str) -> Result<PSID, String> {
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    let rc = unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &mut sid) };
    if rc < 0 || sid.is_null() {
        return Err(format!(
            "DeriveAppContainerSidFromAppContainerName 失败（0x{:08x}）",
            rc
        ));
    }
    Ok(sid)
}

pub(crate) fn free_sid(sid: PSID) {
    // PSID 本身就是 `*mut c_void`：多写一层转换是多余的（clippy 的 unnecessary_cast 点名过）。
    unsafe {
        LocalFree(sid);
    }
}

pub(crate) fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 目的：系统 shell 的绝对路径（容器里显式给镜像路径比让系统搜索可靠）。
pub(crate) fn system_shell() -> PathBuf {
    if let Some(root) = std::env::var_os("SystemRoot") {
        let p = PathBuf::from(root).join("System32").join("cmd.exe");
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("cmd.exe")
}

/// 目的：把自己放进一个 Job Object：最后一个句柄关闭（本进程消亡，含被杀）= 整棵树立即终止。
pub(crate) fn join_kill_on_close_job(max_processes: u32) -> Result<(), String> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return Err("CreateJobObjectW 失败".to_string());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.BasicLimitInformation.ActiveProcessLimit = max_processes;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return Err("SetInformationJobObject 失败".to_string());
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            return Err("AssignProcessToJobObject 失败".to_string());
        }
        // 句柄故意不关：它在守门进程存活期间一直有效（关掉就等于立刻杀自己）。
        Ok(())
    }
}

/// 目的：把工具进程放进容器里跑：不给任何 capability（= 断网），stdio 用外层给的那三个句柄；环境用运行期白名单显式给出。
pub(crate) fn run_in_container(sid: PSID, spec: &FenceSpec, command: &str) -> Result<i32, String> {
    // 环境块显式给（见 fence_env）：子进程拿到的就是白名单本身，不靠"守门进程恰好继承了什么"。
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in super::super::fence_env(spec, command) {
        block.extend(k.encode_wide());
        block.push(u16::from(b'='));
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    let mut caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mut size: usize = 0;
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
    }
    let mut buffer: Vec<u8> = vec![0; size];
    let list = buffer.as_mut_ptr() as *mut c_void;
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err("InitializeProcThreadAttributeList 失败".to_string());
    }
    let ok = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            &mut caps as *mut _ as *mut c_void,
            std::mem::size_of::<SECURITY_CAPABILITIES>(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        unsafe { DeleteProcThreadAttributeList(list) };
        return Err("UpdateProcThreadAttribute 失败".to_string());
    }
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    unsafe {
        si.StartupInfo.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
        si.StartupInfo.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
        si.StartupInfo.hStdError = GetStdHandle(STD_ERROR_HANDLE);
    }
    si.lpAttributeList = list;
    // 命令交系统 shell 解释（与既有语义一致：命令行来自 module.yaml）。
    // 镜像路径显式给出：容器里用 lpApplicationName 解析更稳（不给路径时搜索偶发失败）。
    let shell_exe = system_shell();
    let shell = format!(
        "\"{}\" /C {}",
        shell_exe.display(),
        super::super::windows_program_separators(command)
    );
    let mut cmdline: Vec<u16> = std::ffi::OsStr::new(&shell)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let app = wide(&shell_exe);
    let cwd = wide(&spec.cwd);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            app.as_ptr(),
            cmdline.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr() as *const c_void,
            cwd.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        )
    };
    // 立刻取错误码：DeleteProcThreadAttributeList 会覆盖 GetLastError。
    let err = std::io::Error::last_os_error();
    unsafe { DeleteProcThreadAttributeList(list) };
    if created == 0 {
        return Err(format!(
            "CreateProcessW 失败（{}）：{}",
            shell_exe.display(),
            err
        ));
    }
    let mut code: u32 = FENCE_FAILED as u32;
    unsafe {
        WaitForSingleObject(pi.hProcess, INFINITE);
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    Ok(code as i32)
}
