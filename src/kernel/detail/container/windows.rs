//! 目的：Windows 容器围栏的启动面——AppContainer 档案与 SID、UTF-16 转换、system shell、kill-on-close job 对象。
//! 管：在容器里拉进程这一层（Windows 独有：Linux/macOS 是「先装规则再 exec」）。
//! 不管：授权与撤销（在 acl）、台账（在 ledger）、环境与程序名改写（在 launch）、策略（在 confine::windows::prepare_fence）。
//! 联动：由 confine 的 Windows 后端（prepare_fence / run_fenced）调用；失败如实返回而非假装成功。

use crate::kernel::contracts::fence::{Access, FENCE_FAILED, FencePlan, Reach};
use crate::kernel::detail::io::env;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, DeriveCapabilitySidsFromName, GetLengthSid, PSID, SECURITY_CAPABILITIES,
    SID_AND_ATTRIBUTES, WinCapabilityInternetClientSid,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

mod relay;
pub(crate) use relay::{RELAY_FLAG, relay_entry};

/// 目的：本程序建的容器 profile 前缀（`container_name` 生成的就是它；`--fence-clean` 按它扫整族）。
pub(crate) const PROFILE_PREFIX: &str = "Solomni.Agent.";

/// 目的：容器名：**按"会话 + agent + 授予清单"派生**（授予指纹）——授予固定后同一 agent 的清单恒定，
///   于是同一 (会话, agent) 的后续调用复用同一个 SID 与已有 ACE；清单仍进指纹，防止不同授予集合共用一个身份。
///   工具链/运行包是可信执行基座（授给 baseline），不在指纹里承担范围含义；net 是令牌能力，也不进指纹。
///   外层授权与守门进程各自能算出同一个 SID；其它平台行为不变（这个名字只用于 Windows 的容器身份与日志）。
pub(crate) fn container_name(spec: &FencePlan) -> String {
    // FNV-1a：只为把名字收敛成定长标识（不是安全用途），避免容器名里出现中文与路径。
    // 字段值逐段喂入并补分隔符，避免 ("ab","c") 与 ("a","bc") 撞成同一个身份。
    let mut parts: Vec<String> = vec![
        spec.lease.clone(),
        spec.agent.clone(),
        spec.home.to_string_lossy().into_owned(),
        spec.cwd.to_string_lossy().into_owned(),
    ];
    // 每条授予按"访问档 + 落点形状 + 路径"喂入：只改访问档也要换身份，否则宽的那次授权会被窄的一次复用。
    let mut grants: Vec<String> = spec
        .grants
        .iter()
        .map(|g| {
            let access = match g.access {
                Access::Read => "r",
                Access::Write => "w",
            };
            let reach = match g.reach {
                Reach::Dir => "d",
                Reach::Tree => "t",
            };
            format!(
                "{}{}{}",
                access,
                reach,
                g.path.to_string_lossy().to_ascii_lowercase()
            )
        })
        .collect();
    grants.sort();
    grants.dedup();
    parts.extend(grants);
    parts.push("\u{1}".to_string());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in &parts {
        for b in part.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h ^= 0u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{}{:016x}", PROFILE_PREFIX, h)
}

/// 目的：容器进程的**系统临时目录**（`GetTempPath2` 在 AppContainer 里返回的落点）。
/// 参数：`spec` = 这次执行的围栏规格。
/// 返回：`<私有沙箱>/Packages/<容器名>/AC/Temp`；私有沙箱为空（连 cwd 也没有）时 `None`——没有落点，不猜。
/// 约束：`prepare_fence` 在授权前把它建出来。`GetTempPath2` **不读 TMP/TEMP**：容器里缺了它，
///   编译器这类程序连临时文件都写不了（真机：g++ 直接 abort）。
pub(crate) fn container_temp_dir(spec: &FencePlan) -> Option<PathBuf> {
    container_package_dir(spec).map(|dir| dir.join("AC").join("Temp"))
}

/// 目的：按次容器在私有沙箱里的**包目录**（`<私有沙箱>/Packages/<容器名>`）——临时目录的父级。
/// 返回：私有沙箱为空（连 cwd 也没有）时 `None`。
/// 约束：按次调用结束要连这份目录一起删（profile 被删后它就是空壳）；只在 release_call 里用。
pub(crate) fn container_package_dir(spec: &FencePlan) -> Option<PathBuf> {
    let private = spec.home_or_cwd();
    if private.as_os_str().is_empty() {
        return None;
    }
    Some(private.join("Packages").join(container_name(spec)))
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
/// 约束：profile 侧也要声明产品能力，与令牌侧（run_in_container）用同一个名字。
pub(crate) fn ensure_profile(name: &str) -> Result<(), String> {
    let n: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let d: Vec<u16> = std::ffi::OsStr::new("Solomni 工具围栏")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let capability = product_capability_sid()?;
    let attr = SID_AND_ATTRIBUTES {
        Sid: capability.as_ptr() as PSID,
        Attributes: SE_GROUP_ENABLED,
    };
    let mut sid: PSID = std::ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(n.as_ptr(), n.as_ptr(), d.as_ptr(), &attr, 1, &mut sid)
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
    if let Some(root) = env::system_root() {
        let p = root.join("System32").join("cmd.exe");
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

/// 目的：`SID_AND_ATTRIBUTES.Attributes` 的启用位（winnt.h 的 SE_GROUP_ENABLED）。
const SE_GROUP_ENABLED: u32 = 0x0000_0004;
/// 目的：SID 缓冲上限（winnt.h 的 SECURITY_MAX_SID_SIZE；能力 SID 也在上限内）。
const MAX_SID_BYTES: u32 = 68;

/// 目的：本产品机器级只读授权的能力名——派生成 S-1-15-3-*，只授给声明了它的容器。
///   不用 ALL APPLICATION PACKAGES（S-1-15-2-1）：那会把读权限下放给本机所有 AppContainer 应用。
///   真机探针（tests/windows/probes/capability.rs）已确证：声明了它的容器读得到只授给该能力的文件，
///   不声明的读不到，且两者都写得了自己的工作目录（差异只能归因于能力）。
pub(crate) const CAPABILITY_NAME: &str = "solomni.fence.baseline";

/// 目的：派生本产品的能力 SID（存储自持，调用方不必 LocalFree）。
/// 约束：profile 声明与令牌声明必须用同一个名字，否则 ACE 认不上。
pub(crate) fn product_capability_sid() -> Result<Vec<u8>, String> {
    let name: Vec<u16> = std::ffi::OsStr::new(CAPABILITY_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut groups: *mut PSID = std::ptr::null_mut();
    let mut group_count = 0u32;
    let mut caps: *mut PSID = std::ptr::null_mut();
    let mut cap_count = 0u32;
    let ok = unsafe {
        DeriveCapabilitySidsFromName(
            name.as_ptr(),
            &mut groups,
            &mut group_count,
            &mut caps,
            &mut cap_count,
        )
    };
    if ok == 0 || caps.is_null() || cap_count == 0 {
        return Err(format!(
            "派生产品能力 SID 失败（{}，count={}）",
            CAPABILITY_NAME, cap_count
        ));
    }
    let first = unsafe { *caps };
    let len = unsafe { GetLengthSid(first) } as usize;
    let mut owned = vec![0u8; len];
    unsafe { std::ptr::copy_nonoverlapping(first as *const u8, owned.as_mut_ptr(), len) };
    unsafe {
        LocalFree(caps as *mut c_void);
        if !groups.is_null() {
            LocalFree(groups as *mut c_void);
        }
    }
    Ok(owned)
}

/// 目的：按 `net` 准备容器的网络能力：`true` 只授 `internetClient`（公网出站），`false` 不授（断网）。
///   不授 `privateNetworkClientServer`，也不做 loopback 豁免；返回的 SID 存储必须活到 `CreateProcessW` 返回。
pub(crate) fn internet_client_sid(enabled: bool) -> Result<Option<Vec<u8>>, String> {
    if !enabled {
        return Ok(None);
    }
    let mut sid = vec![0u8; MAX_SID_BYTES as usize];
    let mut size = MAX_SID_BYTES;
    let ok = unsafe {
        CreateWellKnownSid(
            WinCapabilityInternetClientSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr() as PSID,
            &mut size,
        )
    };
    if ok == 0 {
        return Err("CreateWellKnownSid(internetClient) 失败".to_string());
    }
    sid.truncate(size as usize);
    Ok(Some(sid))
}

/// 目的：回环中继档的容器内启动（真现在 `relay` 子模块）。只有 `net.is_local()` 用得到。
pub(crate) fn relay_run(
    sid: PSID,
    spec: &FencePlan,
    command: &str,
    ports: &[u16],
) -> Result<i32, String> {
    relay::run(sid, spec, command, ports)
}

/// 目的：把工具命令包成容器里要跑的整条命令行：交系统 shell 解释，并配上容器里该用的程序（shell 自身）与 cwd。
/// 约束：视图盘只给 cwd == 私有沙箱的调用（shell）用：映射根一级即可，根外祖先链不必再授 ACE，程序名也译视图写法。
///   模块工具 cwd 是模块根，沿用宿主 cwd 与宿主程序名——译成视图写法会把相对程序名算成跨卷绝对路径。
pub(crate) fn container_command(spec: &FencePlan, command: &str) -> (PathBuf, String, PathBuf) {
    let view = crate::kernel::detail::view_drive::active_shell_view(spec);
    let cwd = view
        .as_ref()
        .and_then(|v| v.view_of(&spec.cwd))
        .unwrap_or_else(|| spec.cwd.clone());
    // 程序名改写成相对 cwd 的路径：AppContainer 里绝对路径的程序会被拒（真机：同一文件用绝对路径报
    // Access denied，用 `..\` 相对路径正常）。裸名不碰——交给 cmd 按容器里的 PATH 解析。
    let command = match &view {
        Some(v) => crate::kernel::detail::launch::view_program_command(command, v),
        None => command.to_string(),
    };
    let command = crate::kernel::detail::launch::container_program_command(&command, &cwd);
    // 命令交系统 shell 解释（与既有语义一致：命令行来自 module.yaml）。
    // 镜像路径显式给出：容器里用 lpApplicationName 解析更稳（不给路径时搜索偶发失败）。
    let shell_exe = system_shell();
    let line = format!(
        "\"{}\" /C {}",
        shell_exe.display(),
        crate::kernel::detail::launch::windows_program_separators(&command)
    );
    (shell_exe, line, cwd)
}

/// 目的：在给定容器身份下起一个进程（stdio 继承调用方的三个标准句柄；环境 = 围栏白名单 + `extra_env`）。
/// 参数：`app` 给 `None` 时让 Windows 自己解析命令行首词；`command` 只用来算环境白名单，不参与启动。
/// 返回：起不来的进程句柄（调用方负责 `wait_and_reap`）。
pub(crate) fn spawn_in_container(
    sid: PSID,
    spec: &FencePlan,
    command: &str,
    app: Option<&Path>,
    cmdline: &str,
    cwd: &Path,
    extra_env: &[(String, String)],
) -> Result<PROCESS_INFORMATION, String> {
    // 环境块显式给（见 fence_env）：子进程拿到的就是白名单本身，不靠"守门进程恰好继承了什么"。
    let mut env = crate::kernel::detail::launch::fence_env(spec, command);
    env.extend(
        extra_env
            .iter()
            .map(|(k, v)| (std::ffi::OsString::from(k), std::ffi::OsString::from(v))),
    );
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in env {
        block.extend(k.encode_wide());
        block.push(u16::from(b'='));
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    // 能力数组：产品能力恒在（机器级只读 ACE 授给它，不是授给本机所有 AppContainer 应用）。
    // net=true 再加 internetClient（只公网出站，不授 privateNetworkClientServer、不做 loopback 豁免）；net=false 断网。
    let product = product_capability_sid()?;
    let network_sid = internet_client_sid(spec.net.on())?;
    let mut attrs: Vec<SID_AND_ATTRIBUTES> = vec![SID_AND_ATTRIBUTES {
        Sid: product.as_ptr() as PSID,
        Attributes: SE_GROUP_ENABLED,
    }];
    if let Some(sid) = network_sid.as_ref() {
        attrs.push(SID_AND_ATTRIBUTES {
            Sid: sid.as_ptr() as PSID,
            Attributes: SE_GROUP_ENABLED,
        });
    }
    let mut caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: attrs.as_mut_ptr(),
        CapabilityCount: attrs.len() as u32,
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
    let mut line: Vec<u16> = std::ffi::OsStr::new(cmdline)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let app_wide = app.map(wide);
    let cwd = wide(cwd);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            app_wide
                .as_ref()
                .map(|a| a.as_ptr())
                .unwrap_or(std::ptr::null()),
            line.as_mut_ptr(),
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
        return Err(format!("CreateProcessW 失败（{}）：{}", err, cmdline));
    }
    Ok(pi)
}

/// 目的：等容器里的进程退出并拿退出码，顺手收掉两个句柄。
pub(crate) fn wait_and_reap(pi: PROCESS_INFORMATION) -> Result<i32, String> {
    let mut code: u32 = FENCE_FAILED as u32;
    unsafe {
        WaitForSingleObject(pi.hProcess, INFINITE);
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    Ok(code as i32)
}

/// 目的：把工具进程放进容器里跑（常规档：不经回环中继）。
pub(crate) fn run_in_container(sid: PSID, spec: &FencePlan, command: &str) -> Result<i32, String> {
    let (app, cmdline, cwd) = container_command(spec, command);
    let pi = spawn_in_container(sid, spec, command, Some(&app), &cmdline, &cwd, &[])?;
    wait_and_reap(pi)
}
