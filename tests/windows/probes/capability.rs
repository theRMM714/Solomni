//! 目的：能力 SID 的 OS 事实探针——自定义包能力 SID 声明进 AppContainer 令牌后，能不能满足只授给该能力的 ACE。
//! 管：DeriveCapabilitySidsFromName 派生、profile 与 SECURITY_CAPABILITIES 两处声明、带能力与不带能力的正反对照。
//! 不管：产品的授权策略、台账与撤销（在 src/kernel/detail/confine/）。
//! 联动：tests/windows/probes.rs；node run-tests.js --fence-live。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    BuildTrusteeWithSidW, ConvertSidToStringSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, DeriveCapabilitySidsFromName,
    GetTokenInformation, OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION, PSID,
    SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_READ_ATTRIBUTES,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

/// 目的：本产品要用的能力名（派生成 S-1-15-3-*）；探针与生产必须用同一个名字才有意义。
const CAPABILITY_NAME: &str = "solomni.fence.baseline";
/// 目的：`SID_AND_ATTRIBUTES.Attributes` 的启用位（winnt.h 的 SE_GROUP_ENABLED）。
const SE_GROUP_ENABLED: u32 = 0x0000_0004;
/// 目的：文件对象（SetNamedSecurityInfoW 的对象类型）。
const SE_FILE_OBJECT: i32 = 1;
/// 目的：探针文件里的明文标记——读到它才算越过了能力边界。
const MARKER: &str = "CAPABILITY-ONLY-SECRET";

/// 目的：是否允许跑"会改本机状态"的探针（默认否：测试不该在真机上留下痕迹）。
fn live_enabled() -> bool {
    std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 目的：派生能力 SID 并给出它的文本形式（探针要断言它落在 S-1-15-3-* 这一段）。
/// 返回：(SID 存储, 文本)。存储由调用方 LocalFree。
fn derive_capability() -> Result<(Vec<u8>, String), String> {
    let name = wide(CAPABILITY_NAME);
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
            "DeriveCapabilitySidsFromName 失败（ok={}，count={}）",
            ok, cap_count
        ));
    }
    let first = unsafe { *caps };
    let text = sid_text(first)?;
    // 存储拷一份出来：原数组要整块 LocalFree，不能只留第一个指针。
    let len = unsafe { windows_sys::Win32::Security::GetLengthSid(first) };
    let mut owned = vec![0u8; len as usize];
    unsafe { std::ptr::copy_nonoverlapping(first as *const u8, owned.as_mut_ptr(), len as usize) };
    unsafe {
        LocalFree(caps as *mut c_void);
        if !groups.is_null() {
            LocalFree(groups as *mut c_void);
        }
    }
    Ok((owned, text))
}

/// 目的：SID → 文本（转换失败如实报错，不返回占位串）。
fn sid_text(sid: PSID) -> Result<String, String> {
    let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut raw) } == 0 || raw.is_null() {
        return Err("ConvertSidToStringSidW 失败".to_string());
    }
    let text = unsafe {
        let mut n = 0usize;
        while *raw.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(raw, n))
    };
    unsafe { LocalFree(raw as *mut c_void) };
    Ok(text)
}

/// 目的：当前用户的 SID（探针目录要留给宿主自己收拾，所以 DACL 里必须带上它）。
fn current_user() -> Result<Vec<u8>, String> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err("打开进程令牌失败".to_string());
    }
    let mut len = 0u32;
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        unsafe { CloseHandle(token) };
        return Err("取令牌用户长度失败".to_string());
    }
    let mut buf = vec![0u8; len as usize];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        )
    };
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err("取令牌用户失败".to_string());
    }
    let tu = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let n = unsafe { windows_sys::Win32::Security::GetLengthSid(tu.User.Sid) };
    let mut owned = vec![0u8; n as usize];
    unsafe {
        std::ptr::copy_nonoverlapping(tu.User.Sid as *const u8, owned.as_mut_ptr(), n as usize)
    };
    Ok(owned)
}

/// 目的：一条要写进 DACL 的授权（主体 + 权限位 + 继承位）。
struct Entry {
    sid: PSID,
    mask: u32,
    inherit: u32,
}

/// 目的：给一个路径写**受保护**的 DACL（切断父目录继承）——探针要确证「只有能力 SID 能读」，
///   就不能让 TEMP 上继承来的任何条目混进判据。
fn set_dacl(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let mut eas: Vec<EXPLICIT_ACCESS_W> = Vec::new();
    for e in entries {
        let mut trustee: TRUSTEE_W = unsafe { std::mem::zeroed() };
        unsafe { BuildTrusteeWithSidW(&mut trustee, e.sid) };
        trustee.TrusteeForm = TRUSTEE_IS_SID;
        trustee.TrusteeType = TRUSTEE_IS_UNKNOWN;
        eas.push(EXPLICIT_ACCESS_W {
            grfAccessPermissions: e.mask,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: e.inherit,
            Trustee: trustee,
        });
    }
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let rc =
        unsafe { SetEntriesInAclW(eas.len() as u32, eas.as_ptr(), std::ptr::null(), &mut dacl) };
    if rc != 0 || dacl.is_null() {
        return Err(format!("拼 ACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let w = wide_path(path);
    let rc = unsafe {
        SetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        )
    };
    unsafe { LocalFree(dacl as *mut c_void) };
    if rc != 0 {
        return Err(format!("写 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    Ok(())
}

/// 目的：建（或复用）一个 AppContainer profile，**按 with_capability 决定要不要声明能力**。
///   这是正反对照的唯一自变量之一：profile 侧的声明。
fn ensure_profile(name: &str, cap: Option<PSID>) -> Result<(), String> {
    let n = wide(name);
    let d = wide("Solomni 能力 SID 探针");
    let mut attr = cap.map(|sid| SID_AND_ATTRIBUTES {
        Sid: sid,
        Attributes: SE_GROUP_ENABLED,
    });
    let (ptr, count) = match attr.as_mut() {
        Some(a) => (a as *mut SID_AND_ATTRIBUTES, 1u32),
        None => (std::ptr::null_mut(), 0u32),
    };
    let mut sid: PSID = std::ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(n.as_ptr(), n.as_ptr(), d.as_ptr(), ptr, count, &mut sid)
    };
    const E_ALREADY_EXISTS: i32 = 0x8007_00B7u32 as i32;
    if !sid.is_null() {
        unsafe { LocalFree(sid) };
    }
    if hr >= 0 || hr == E_ALREADY_EXISTS {
        return Ok(());
    }
    Err(format!(
        "CreateAppContainerProfile 失败（0x{:08x}）",
        hr as u32
    ))
}

/// 目的：派生容器 SID（存储由调用方 LocalFree）。
fn container_sid(name: &str) -> Result<Vec<u8>, String> {
    let n = wide(name);
    let mut sid: PSID = std::ptr::null_mut();
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(n.as_ptr(), &mut sid) };
    if hr < 0 || sid.is_null() {
        return Err(format!("派生容器 SID 失败（0x{:08x}）", hr as u32));
    }
    let len = unsafe { windows_sys::Win32::Security::GetLengthSid(sid) };
    let mut owned = vec![0u8; len as usize];
    unsafe { std::ptr::copy_nonoverlapping(sid as *const u8, owned.as_mut_ptr(), len as usize) };
    unsafe { LocalFree(sid) };
    Ok(owned)
}

/// 目的：在容器里跑一条 cmd 命令，**能力数组由调用方给**（这是正反对照的另一个自变量：令牌侧的声明）。
/// 返回：进程退出码。stdio 直通，环境块给一个最小的白名单。
fn run(sid: PSID, caps: &[PSID], cwd: &Path, command: &str) -> Result<u32, String> {
    let mut attrs: Vec<SID_AND_ATTRIBUTES> = caps
        .iter()
        .map(|s| SID_AND_ATTRIBUTES {
            Sid: *s,
            Attributes: SE_GROUP_ENABLED,
        })
        .collect();
    let mut sc = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: if attrs.is_empty() {
            std::ptr::null_mut()
        } else {
            attrs.as_mut_ptr()
        },
        CapabilityCount: attrs.len() as u32,
        Reserved: 0,
    };
    let mut size = 0usize;
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    let mut buffer = vec![0u8; size];
    let list = buffer.as_mut_ptr() as *mut c_void;
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err("InitializeProcThreadAttributeList 失败".to_string());
    }
    let ok = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            &mut sc as *mut _ as *mut c_void,
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
    let shell = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let shell_exe = PathBuf::from(shell).join("System32").join("cmd.exe");
    let cmdline = wide(&format!("\"{}\" /C {}", shell_exe.display(), command));
    let app = wide_path(&shell_exe);
    let dir = wide_path(cwd);
    // 环境块显式给：LOCALAPPDATA 缺了 CreateProcessW 直接报 os error 203
    //（与 src/kernel/detail/confine/paths.rs 的 fence_env 同一条约束）。
    let mut block: Vec<u16> = Vec::new();
    let mut push = |k: &str, v: &str| {
        block.extend(std::ffi::OsStr::new(k).encode_wide());
        block.push(u16::from(b'='));
        block.extend(std::ffi::OsStr::new(v).encode_wide());
        block.push(0);
    };
    for k in [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "SYSTEMDRIVE",
    ] {
        if let Some(v) = std::env::var_os(k) {
            push(k, &v.to_string_lossy());
        }
    }
    let home = cwd.display().to_string();
    for k in ["LOCALAPPDATA", "USERPROFILE", "HOME", "TEMP", "TMP"] {
        push(k, &home);
    }
    block.push(0);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let mut cmd = cmdline;
    let created = unsafe {
        CreateProcessW(
            app.as_ptr(),
            cmd.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr() as *const c_void,
            dir.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        )
    };
    let err = std::io::Error::last_os_error();
    unsafe { DeleteProcThreadAttributeList(list) };
    if created == 0 {
        return Err(format!("CreateProcessW 失败：{}", err));
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(pi.hProcess, INFINITE);
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    Ok(code)
}

/// 目的：探针目录（用后即删，不在本机留痕）。
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solomni-capability-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建探针目录");
    dir
}

/// 目的：【真机·OS 事实】自定义包能力 SID 能不能当授权主体用。
///   正向：只授给能力 SID 的文件，声明了该能力的容器读得到。
///   反向：同一个文件，**不声明**该能力的容器读不到（两个容器 SID 都在 work 上有写权，
///   所以「读不到」只能归因于能力，不是归因于进程没起来或 cwd 进不去）。
/// 会创建 AppContainer profile 与写目录 ACL（改本机状态），只在 --fence-live 下跑。
#[test]
fn capability_sid_is_grantable_and_not_blanket() {
    if !live_enabled() {
        eprintln!(
            "[探针] 未开启真机围栏测试：capability_sid_is_grantable_and_not_blanket 会创建 AppContainer profile 与写 ACL（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    let (cap, cap_text) = derive_capability().expect("派生能力 SID");
    assert!(
        cap_text.starts_with("S-1-15-3-"),
        "能力 SID 应落在 S-1-15-3-* 段，实际 {}",
        cap_text
    );
    eprintln!("[诊断] 能力名 {} → {}", CAPABILITY_NAME, cap_text);
    let cap_sid = cap.as_ptr() as PSID;
    let user = current_user().expect("取当前用户 SID");
    let user_sid = user.as_ptr() as PSID;

    let base = scratch();
    let probe = base.join("probe");
    std::fs::create_dir_all(&probe).expect("建探针根");
    let with_name = format!("Solomni.Probe.Cap.With.{}", std::process::id());
    let without_name = format!("Solomni.Probe.Cap.Without.{}", std::process::id());
    let result =
        std::panic::catch_unwind(|| run_case(&probe, cap_sid, user_sid, &with_name, &without_name));
    // 无论断言成败都要收尾：profile 与目录都不留在本机。
    let _ = unsafe { DeleteAppContainerProfile(wide(&with_name).as_ptr()) };
    let _ = unsafe { DeleteAppContainerProfile(wide(&without_name).as_ptr()) };
    let _ = std::fs::remove_dir_all(&base);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// 目的：正反对照的主体（与收尾分开，保证 panic 也能清理）。
fn run_case(probe: &Path, cap_sid: PSID, user_sid: PSID, with_name: &str, without_name: &str) {
    ensure_profile(with_name, Some(cap_sid)).expect("建带能力的 profile");
    ensure_profile(without_name, None).expect("建不带能力的 profile");
    let with_sid = container_sid(with_name).expect("派生带能力容器 SID");
    let without_sid = container_sid(without_name).expect("派生不带能力容器 SID");
    let with_ptr = with_sid.as_ptr() as PSID;
    let without_ptr = without_sid.as_ptr() as PSID;

    // 先写 ACE 再填充：secret.txt 建出来就只带能力 SID 那一条继承 ACE。
    // 两个容器 SID 在 probe 上只拿读属性（够解析 cwd），不拿内容读——内容读只来自能力。
    set_dacl(
        probe,
        &[
            Entry {
                sid: user_sid,
                mask: FILE_ALL_ACCESS,
                inherit: OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            },
            Entry {
                sid: cap_sid,
                mask: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                inherit: OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            },
            Entry {
                sid: with_ptr,
                mask: FILE_READ_ATTRIBUTES,
                inherit: 0,
            },
            Entry {
                sid: without_ptr,
                mask: FILE_READ_ATTRIBUTES,
                inherit: 0,
            },
        ],
    )
    .expect("写探针根 DACL");
    let secret = probe.join("secret.txt");
    std::fs::write(&secret, MARKER).expect("写探针明文");
    let work = probe.join("work");
    std::fs::create_dir_all(&work).expect("建容器工作目录");
    set_dacl(
        &work,
        &[
            Entry {
                sid: user_sid,
                mask: FILE_ALL_ACCESS,
                inherit: OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            },
            Entry {
                sid: with_ptr,
                mask: FILE_ALL_ACCESS,
                inherit: OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            },
            Entry {
                sid: without_ptr,
                mask: FILE_ALL_ACCESS,
                inherit: OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            },
        ],
    )
    .expect("写工作目录 DACL");

    let command = "type ..\\secret.txt > out.txt 2>&1";
    let read_out = |name: &str| std::fs::read_to_string(work.join(name)).unwrap_or_default();

    // 先确证两个容器都真的起来了、且写得了自己的工作目录：否则后面的「读不到」是假结论。
    for (label, sid, caps) in [
        ("with", with_ptr, vec![cap_sid]),
        ("without", without_ptr, Vec::new()),
    ] {
        let alive = format!("echo alive > alive-{}.txt 2>&1", label);
        let code = run(sid, &caps, &work, &alive).expect("容器应当能启动");
        let got = read_out(&format!("alive-{}.txt", label));
        eprintln!(
            "[诊断] {}容器写工作目录：exit={} 拿到 {:?}；work 条目 {:?}",
            label,
            code,
            got.trim(),
            std::fs::read_dir(&work)
                .map(|it| it
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>())
                .unwrap_or_default()
        );
        assert!(
            code == 0 && got.contains("alive"),
            "{}容器必须真的起来且写得了工作目录（exit={}，拿到 {:?}）",
            label,
            code,
            got.trim()
        );
    }

    // 正向：声明了能力的容器应当读得到只授给该能力的文件。
    let code = run(with_ptr, &[cap_sid], &work, command).expect("带能力容器应当能启动");
    let got = read_out("out.txt");
    eprintln!(
        "[诊断] 带能力容器读能力专属文件：exit={} 拿到 {:?}",
        code,
        got.trim()
    );
    assert!(
        code == 0 && got.contains(MARKER),
        "声明了能力 SID 的容器要读得到只授给该能力的文件（exit={}，拿到 {:?}）：能力 SID 不能当授权主体用，方案要回退",
        code,
        got.trim()
    );

    // 反向：不声明能力的容器读不到（上面已确证它写得进 work，所以差异只能归因于能力）。
    let _ = std::fs::remove_file(work.join("out.txt"));
    let code = run(without_ptr, &[], &work, command).expect("不带能力容器应当能启动");
    let got = read_out("out.txt");
    assert!(
        !got.contains(MARKER),
        "不声明该能力的容器不该读得到（exit={}，拿到 {:?}）：授权面没有收窄到本产品",
        code,
        got.trim()
    );
    eprintln!(
        "[诊断] 能力 SID 正反对照成立：带能力读得到、不带能力读不到（exit={}）",
        code
    );
}
