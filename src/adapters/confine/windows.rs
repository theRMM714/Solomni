//! Windows 后端：AppContainer（文件系统与网络围栏）+ Job Object（进程树围栏）。
//! 机制：按 agent 派生一个容器 SID → 把「可达范围」逐条授权给它
//! （共享区与私有沙箱读写、模块目录读写、解释器安装目录只读+执行）
//! → 用 STARTUPINFOEX 的 SECURITY_CAPABILITIES 启动工具（**不给任何 capability = 默认断网**）。
//! 祖先目录不用授权：容器令牌自带 SeChangeNotifyPrivilege（绕过遍历检查），按名走到被放行的根不需要 FILE_TRAVERSE。
//! 授权只落在用户自己拥有的目录上（不需要管理员）；撤销用同一套机制反向做。
//! 授权与撤销由外层进程做（见 confine::prepare_fence），一次性做好并记在会话内存里；
//! 守门进程只负责"按同一个名字派生同一个 SID 并把工具放进去"。
//! 机制不可用时一律如实降级（stderr 说明 + 启动报告 fs/net=false），绝不假装有围栏。

use super::{shell_command, Capability, FenceVerdict, FENCE_FAILED};
use crate::core::fence::FenceSpec;
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SetEntriesInAclW,
    SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS, REVOKE_ACCESS, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetAclInformation, ACL, ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE,
    DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE, PSID, SECURITY_CAPABILITIES,
};
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
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
const SE_FILE_OBJECT: i32 = 1;
// 权限位**只用具体位**：通用位（GENERIC_READ / WRITE / EXECUTE / ALL）的常量值极易记错，写错一个给出去的
// 就是完全不同的权限。真机上抓到过：标着 GENERIC_READ 的是 0x4000_0000（其实是 GENERIC_WRITE）、
// 标着 GENERIC_EXECUTE 的是 0x1000_0000（其实是 GENERIC_ALL）——于是"只读"的解释器基线实际授出了全权。
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
/// 目录里建/删子项（工具要能重写自己的产物）。
const FILE_DELETE_CHILD: u32 = 0x0000_0040;
const DELETE: u32 = 0x0001_0000;
const WRITE_DAC: u32 = 0x0004_0000;
/// 通用位：ACL 里存的可能是它们，也可能是内核展开后的具体位，比较覆盖关系时两者等价。
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_EXECUTE: u32 = 0x2000_0000;
const GENERIC_ALL: u32 = 0x1000_0000;

/// 数据边界（会话目录、模块目录）→ 读写 + 删子项 + 写 DACL（撤权要用）。
const RIGHTS_RW: u32 = FILE_GENERIC_READ
    | FILE_GENERIC_WRITE
    | FILE_GENERIC_EXECUTE
    | FILE_DELETE_CHILD
    | DELETE
    | WRITE_DAC;
/// 只读 + 执行（解释器安装目录：脚本要跑就得读得到它）。
const RIGHTS_RO: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;

/// 一次工具执行最多这么多进程（含 shell 与它拉起的子进程）。
const MAX_PROCESSES: u32 = 32;

extern "system" {
    /// 递归给整棵树设 DACL：SDK 头文件在部分版本里标了废弃，但 advapi32 始终导出（本机已实测）。
    fn TreeSetNamedSecurityInfoW(
        object_name: *const u16,
        object_type: u32,
        security_info: u32,
        owner: PSID,
        group: PSID,
        dacl: *mut ACL,
        sacl: *mut ACL,
        action: u32,
        progress: *mut c_void,
        invoke: u32,
        args: *mut c_void,
    ) -> u32;
}

/// TREE_SEC_INFO_SET（递归设置）。
const TREE_SEC_INFO_SET: u32 = 1;
/// ProgressInvokeNever（不回调进度）。
const PROGRESS_INVOKE_NEVER: u32 = 1;

pub fn capability() -> Capability {
    match self_check() {
        Ok(()) => Capability {
            fs: true,
            net: true,
            tree: true,
            note: "Windows：AppContainer（文件系统可达范围 + 默认断网）与 Job Object（进程树）都由内核强制".to_string(),
        },
        Err(e) => Capability {
            fs: false,
            net: false,
            tree: true,
            note: format!("Windows：容器围栏装不上（{}）——只有进程树围栏与环境白名单，如实降级", e),
        },
    }
}

/// 自检：派生 SID + 真去改一个目录的 DACL（改不动就说明本机环境不允许，如实报 fs=false）。
fn self_check() -> Result<(), String> {
    let sid = container_sid("Solomni.Fence.SelfCheck")?;
    let scratch =
        std::env::temp_dir().join(format!("solomni-fence-selfcheck-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| format!("建自检目录失败：{}", e))?;
    let outcome = grant_one(sid, &scratch, RIGHTS_RO, false, false);
    let _ = std::fs::remove_dir_all(&scratch);
    free_sid(sid);
    outcome.map_err(|e| format!("改不动目录 ACL：{}", e))
}

/// 本程序建的容器 profile 前缀（`container_name` 生成的就是它；`--fence-clean` 按它扫整族）。
const PROFILE_PREFIX: &str = "Solomni.Agent.";

/// 容器名：**只由 agent 名决定**——一个 agent 一个 profile，跨会话复用（数量有界），
/// 外层授权与守门进程因此各自能算出同一个 SID。各会话之间的路径隔离仍由那些目录上的 ACE 决定
/// （同名 agent 的多个会话共用一个容器身份，这是"数量有界"换来的取舍）。
fn container_name(spec: &FenceSpec) -> String {
    // FNV-1a：只为把名字收敛成定长标识（不是安全用途），避免容器名里出现中文与路径。
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in spec.agent.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{}{:016x}", PROFILE_PREFIX, h)
}

/// 名字是不是本程序建过的容器 profile（Windows 把包目录名转成小写，所以按不区分大小写比）。
fn is_our_profile(name: &str) -> bool {
    name.get(..PROFILE_PREFIX.len())
        .map(|head| head.eq_ignore_ascii_case(PROFILE_PREFIX))
        .unwrap_or(false)
}

/// 环境不允许容器围栏时的标记（探针据此区分"环境不允许"与"代码有问题"，不互相顶包）。
pub const ENV_BLOCKED_MARK: &str = "容器围栏不可用（本环境不允许";

/// 建（或复用）容器 profile：**必须有 profile** —— 没有 profile 的派生 SID 拿不到
/// ALL APPLICATION PACKAGES 组，连系统目录里的 cmd.exe 都打不开（实测会报"找不到文件"）。
/// 已存在 = 成功（同一个名字派生出的 SID 与 profile 一致，所以外层用 Derive 预授权 ACL 也有效）。
fn ensure_profile(name: &str) -> Result<(), String> {
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
            "0x{:08x}（拒绝访问：本环境不允许建 AppContainer profile）",
            hr as u32
        ));
    }
    Err(format!("0x{:08x}", hr as u32))
}

/// 派生容器 SID（本机实测：非管理员可用）。
fn container_sid(name: &str) -> Result<PSID, String> {
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

fn free_sid(sid: PSID) {
    // PSID 本身就是 `*mut c_void`：多写一层转换是多余的（clippy 的 unnecessary_cast 点名过）。
    unsafe {
        LocalFree(sid);
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 给一个对象授一条 ACE。`recursive` = 连**已有**子项一起设成这个 ACL（TreeSet）；`inherit` = 这条 ACE 被**新建**子项继承。
/// 两者都要有明确理由：`inherit` 会牵动整棵子树的继承计算（真机实测：2000 个子项的可继承 ACE 写入是空目录的
/// 20 倍），`recursive` 更是会把整棵树设一遍 ACL——所以只对**真的需要被子项继承**的落点（解释器目录、数据边界）用。
fn grant_one(
    sid: PSID,
    path: &Path,
    rights: u32,
    recursive: bool,
    inherit: bool,
) -> Result<(), String> {
    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("读 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: rights,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: if inherit {
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
        } else {
            0
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let rc = unsafe { SetEntriesInAclW(1, &ea, old_dacl as *const ACL, &mut new_dacl) };
    if rc != 0 || new_dacl.is_null() {
        unsafe {
            LocalFree(sd);
        }
        return Err(format!("拼 ACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let rc = if recursive {
        unsafe {
            TreeSetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT as u32,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
                TREE_SEC_INFO_SET,
                std::ptr::null_mut(),
                PROGRESS_INVOKE_NEVER,
                std::ptr::null_mut(),
            )
        }
    } else {
        unsafe {
            SetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl as *const ACL,
                std::ptr::null(),
            )
        }
    };
    unsafe {
        LocalFree(new_dacl as *mut c_void);
        LocalFree(sd);
    }
    if rc != 0 {
        return Err(format!("写 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    Ok(())
}

/// 把该 SID 的 ACE 从对象上撤掉（会话删除时清理用）。
fn revoke_one(sid: PSID, path: &Path, recursive: bool) -> Result<(), String> {
    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(format!("读 DACL 失败（{}）：错误码 {}", path.display(), rc));
    }
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: 0,
        grfAccessMode: REVOKE_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    };
    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let rc = unsafe { SetEntriesInAclW(1, &ea, old_dacl as *const ACL, &mut new_dacl) };
    if rc != 0 {
        unsafe {
            LocalFree(sd);
        }
        return Err(format!(
            "拼撤销后的 ACL 失败（{}）：错误码 {}",
            path.display(),
            rc
        ));
    }
    let rc = if recursive {
        unsafe {
            TreeSetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT as u32,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
                TREE_SEC_INFO_SET,
                std::ptr::null_mut(),
                PROGRESS_INVOKE_NEVER,
                std::ptr::null_mut(),
            )
        }
    } else {
        unsafe {
            SetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl as *const ACL,
                std::ptr::null(),
            )
        }
    };
    unsafe {
        if !new_dacl.is_null() {
            LocalFree(new_dacl as *mut c_void);
        }
        LocalFree(sd);
    }
    if rc != 0 {
        return Err(format!(
            "写撤权后的 DACL 失败（{}）：错误码 {}",
            path.display(),
            rc
        ));
    }
    Ok(())
}

/// 命令里解释器的安装目录：共用实现在 confine/mod.rs（Windows 的目录 ACL 与 macOS 的 seatbelt 同一套语义）。
fn interpreter_dirs(command: &str) -> Vec<PathBuf> {
    super::interpreter_dirs(command)
}

/// 外层进程调用：把围栏要用的授权一次性做好（按 (SID, 路径, 权限) 去重，不重复改 ACL）。
/// 授权落点只有两处：共享区/私有沙箱/模块目录（读写）、解释器安装目录（只读+执行）。
pub fn prepare_fence(
    spec: &FenceSpec,
    command: &str,
    prepared: &std::sync::Mutex<BTreeSet<String>>,
    home: &Path,
) -> Result<(), String> {
    let mut result = Ok(());
    // 这一轮真正写下去的授权（用于如实打印足迹 + 落台账，供 --fence-clean 精确回收）。
    let mut written: Vec<(String, PathBuf, u32)> = Vec::new();
    // 基线都授给 ALL APPLICATION PACKAGES（与 agent 无关）：已有**够用**的 ACE 就跳过，第一次之后不再重走整棵树。
    let base = baseline_sid()?;
    let interpreters = interpreter_dirs(command);
    // 基线一：解释器安装目录（只读+执行）——**必需**：拿不到它，容器里连解释器都起不来。
    for dir in interpreters.iter().cloned() {
        if has_ace_for(base, &dir, RIGHTS_RO) {
            continue;
        }
        if let Err(e) = grant_one(base, &dir, RIGHTS_RO, true, true) {
            eprintln!("[围栏] 解释器目录授权未完成（{}）：{}", dir.display(), e);
            if result.is_ok() {
                result = Err(e);
            }
        } else {
            written.push((String::from("S-1-15-2-1"), dir, RIGHTS_RO));
        }
    }
    // **祖先链不用授**：容器的令牌里有 SeChangeNotifyPrivilege（Bypass traverse checking，真机 whoami /priv
    // 确认 Enabled），按名走到被放行的根不需要祖先上的 FILE_TRAVERSE。以前那一趟"给祖先授穿过"只在改写
    // C:\、C:\Users 这种巨型目录的 DACL 时付出代价——Windows 会顺着整棵树重算继承，真机实测 ~90 s/条
    // （CI 上两条 ACL 契约测试各 95 s，就是它）。删掉这一趟：授权面更小，也不再碰产品目录之外的系统目录。
    free_sid(base);

    // 数据边界（会话目录、模块目录）→ 只授权**叶子本身**，授给该 agent 自己的容器 SID（互相看不见）。
    // 祖先链由上面的基线负责，所以这一层不随会话数量增长。
    let sid = container_sid(&container_name(spec))?;
    let mut todo: Vec<(PathBuf, u32, bool)> = Vec::new();
    for root in spec.rw.iter().chain(std::iter::once(&spec.cwd)) {
        if root.as_os_str().is_empty() {
            continue;
        }
        todo.push((root.clone(), RIGHTS_RW, true));
    }
    // 用户显式授权的只读根（`fence_read`）：只写只读 ACE，**授给该 agent 自己的容器 SID**。
    // 不能像解释器基线那样授给共享组（S-1-15-2-1）——那等于把用户数据开放给机器上任意容器程序。
    // 只读根不递归：用户可能授一个很大的目录（例如项目根），递归会改整棵树的 DACL。
    for root in &spec.ro {
        if root.as_os_str().is_empty() {
            continue;
        }
        todo.push((root.clone(), RIGHTS_RO, false));
    }
    for (path, rights, recursive) in todo {
        let key = format!("{:?}|{}|{}", sid, path.to_string_lossy(), rights);
        if prepared.lock().expect("授权表锁").contains(&key) {
            continue;
        }
        match grant_one(sid, &path, rights, recursive, true) {
            Ok(()) => {
                prepared.lock().expect("授权表锁").insert(key);
                written.push((sid_to_string(sid), path.clone(), rights));
            }
            Err(e) => {
                // 一个落点授不上（例如祖先里的系统目录）不整体失败：如实记下，让自检与探针去判定。
                eprintln!("[围栏] 授权未完成：{}", e);
                if result.is_ok() {
                    result = Err(e);
                }
            }
        }
    }
    let container = container_name(spec);
    free_sid(sid);
    if !written.is_empty() {
        // 如实打印足迹：用户看得见到底动了哪些目录、授给了谁。
        let list: Vec<String> = written
            .iter()
            .map(|(s, p, _)| format!("{} → {}", s, p.display()))
            .collect();
        eprintln!("[围栏] 已写权限 {} 处：{}", written.len(), list.join("；"));
        if let Err(e) = record_grants(home, &container, &written) {
            eprintln!(
                "[围栏] 授权台账落盘失败（影响 --fence-clean 的精确回收）：{}",
                e
            );
        }
    }
    result
}

/// 授权台账：记下"我们给谁、在哪些路径上写了权限"，`--fence-clean` 按它精确回收。
/// 位置：产品私有区 `.home/fence-grants.json`（数据不出工作区）。
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct GrantRecord {
    /// 我们创建过的容器 profile 名（清理时按名删除）。
    #[serde(default)]
    profiles: BTreeSet<String>,
    /// (SID, 路径, 权限位) —— 逐条对应写下去的 ACE。
    #[serde(default)]
    grants: Vec<(String, String, u32)>,
}

fn record_path(home: &Path) -> PathBuf {
    home.join("fence-grants.json")
}

fn load_record(home: &Path) -> GrantRecord {
    std::fs::read_to_string(record_path(home))
        .ok()
        .and_then(|t| serde_json::from_str::<GrantRecord>(&t).ok())
        .unwrap_or_default()
}

fn save_record(home: &Path, rec: &GrantRecord) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("建私有区失败：{}", e))?;
    let text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
    std::fs::write(record_path(home), text).map_err(|e| e.to_string())
}

/// 记下"我们建过这个容器 profile"（与 `prepare_fence` 共用同一份台账），供 `--fence-clean` 精确回收。
fn record_profile(home: &Path, name: &str) {
    let mut rec = load_record(home);
    if rec.profiles.insert(name.to_string()) {
        if let Err(e) = save_record(home, &rec) {
            eprintln!(
                "[围栏] 容器 profile 台账落盘失败（影响 --fence-clean 的精确回收）：{}",
                e
            );
        }
    }
}

/// 扫掉本程序建过的整族容器 profile：台账只记"我们知道写过什么"，而 profile 可能来自没有台账的路径
/// （探针、夹具的台账被删、旧版本）。名字前缀是本程序独有的，所以按它扫；`DeleteAppContainerProfile`
/// 连该容器的存储一起删。返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    let root = match std::env::var_os("LOCALAPPDATA") {
        Some(v) => PathBuf::from(v).join("Packages"),
        None => return Err("取不到 LOCALAPPDATA（容器 profile 的存储根）".to_string()),
    };
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        // 没有 Packages 目录 = 本机没有容器 profile。
        Err(_) => return Ok(0),
    };
    let mut deleted = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_our_profile(&name) {
            continue;
        }
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        if unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0 {
            deleted += 1;
        }
    }
    Ok(deleted)
}

fn record_grants(
    home: &Path,
    container: &str,
    written: &[(String, PathBuf, u32)],
) -> Result<(), String> {
    let mut rec = load_record(home);
    rec.profiles.insert(container.to_string());
    for (sid, path, rights) in written {
        let entry = (sid.clone(), path.to_string_lossy().into_owned(), *rights);
        if !rec.grants.contains(&entry) {
            rec.grants.push(entry);
        }
    }
    save_record(home, &rec)
}

/// SID → 字符串（写台账用）。
fn sid_to_string(sid: PSID) -> String {
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

/// 字符串 → SID（清理时按台账里的字符串还原）。
fn sid_from_string(text: &str) -> Result<PSID, String> {
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

/// 精确回收：按台账把我们写过的 ACE 逐条撤掉，并删掉我们建过的容器 profile。
/// 返回给用户看的一句话（清理了几条、删了几个 profile）。
pub fn clean(home: &Path) -> Result<String, String> {
    let rec = load_record(home);
    if rec.grants.is_empty() && rec.profiles.is_empty() {
        return Ok("没有台账：本程序没在本机写过权限项".to_string());
    }
    let mut removed = 0usize;
    for (sid_text, path, _rights) in &rec.grants {
        let sid = match sid_from_string(sid_text) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[围栏] {}", e);
                continue;
            }
        };
        let p = PathBuf::from(path);
        if p.exists() {
            match revoke_one(sid, &p, false) {
                Ok(()) => removed += 1,
                Err(e) => eprintln!("[围栏] 撤销未完成：{}", e),
            }
        }
        free_sid(sid);
    }
    let mut deleted = 0usize;
    for name in &rec.profiles {
        let n: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let hr = unsafe { DeleteAppContainerProfile(n.as_ptr()) };
        if hr >= 0 {
            deleted += 1;
        }
    }
    let _ = std::fs::remove_file(record_path(home));
    Ok(format!(
        "已撤销 {} 条授权、删除 {} 个容器 profile",
        removed, deleted
    ))
}

/// 撤销一次会话的授权（会话删除时经 FenceHost 端口调用）。
/// home 为 None 时只撤权限、不动台账（无台的调用方少见；正常路径都给 home）。
pub fn release_fence_home(spec: &FenceSpec, home: Option<&Path>) -> Result<(), String> {
    let sid = container_sid(&container_name(spec))?;
    let mut result = Ok(());
    // 撤权要覆盖**同一次授权写下的全部条目**：读写根、只读根与工作目录。
    let mut paths: Vec<PathBuf> = spec.rw.clone();
    paths.extend(spec.ro.iter().cloned());
    paths.push(spec.cwd.clone());
    paths.sort();
    paths.dedup();
    // 台账比对用字符串：下面 paths 会被消费掉。
    let path_texts: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for p in paths {
        if p.as_os_str().is_empty() {
            continue;
        }
        if let Err(e) = revoke_one(sid, &p, true) {
            eprintln!("[围栏] 撤销未完成：{}", e);
            if result.is_ok() {
                result = Err(e);
            }
        }
    }
    let sid_text = sid_to_string(sid);
    free_sid(sid);
    if let Some(h) = home {
        // 台账跟着会话一起清：撤掉的条目不留残账（账目等于"本机现在还有我们写的哪些权限"）。
        let mut rec = load_record(h);
        rec.grants
            .retain(|(s, p, _)| s != &sid_text || !path_texts.iter().any(|x| x == p));
        if let Err(e) = save_record(h, &rec) {
            eprintln!("[围栏] 台账更新失败：{}", e);
        }
    }
    result
}

/// 兼容旧调用点：不带台账的撤销。
pub fn release_fence(spec: &FenceSpec) -> Result<(), String> {
    release_fence_home(spec, None)
}

/// 只读运行基线的授权对象：ALL APPLICATION PACKAGES（S-1-15-2-1）。
/// 我们的容器令牌本来就带这个组，所以「解释器与系统只读区」这类基线只授一次、与 agent 身份无关；
/// 每 agent 一个的容器 SID 只用来圈**数据边界**（会话目录、模块目录）。
fn baseline_sid() -> Result<PSID, String> {
    let s: Vec<u16> = std::ffi::OsStr::new("S-1-15-2-1")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(s.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err("取不到 ALL APPLICATION PACKAGES 的 SID".to_string());
    }
    Ok(sid)
}

/// 把通用位展开成具体位：ACL 里存的是哪一套，覆盖关系比较都要等价成立。
fn expand_generics(mask: u32) -> u32 {
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

/// 已有 ACE 的权限位是不是覆盖得住我们需要的权限位。
fn rights_covered(mask: u32, rights: u32) -> bool {
    expand_generics(rights) & !expand_generics(mask) == 0
}

/// 该对象上是不是已经有给这个 SID 的允许 ACE，**且权限位覆盖得住**。
/// 用途：基线授权只以递归方式写过一次，所以根上已有"够用"的 ACE 就跳过整棵树——否则每来一个 agent 都要重走几万文件。
/// 只看"有没有该 SID 的 ACE"不够：真机上解释器目录继承了只有 SYNCHRONIZE 的 ALL APPLICATION PACKAGES ACE，
/// 基线因此被整条跳过，容器里连解释器都读不到（工具报 python is not recognized）。
fn has_ace_for(sid: PSID, path: &Path, rights: u32) -> bool {
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACL_SIZE_INFORMATION_CLASS: i32 = 2;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 || dacl.is_null() {
        return false;
    }
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            ACL_SIZE_INFORMATION_CLASS,
        )
    };
    let mut found = false;
    if ok != 0 {
        for i in 0..info.AceCount {
            let mut ace: *mut c_void = std::ptr::null_mut();
            if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
                continue;
            }
            let base = ace as *const u8;
            // ACCESS_ALLOWED_ACE：AceType(1) + AceFlags(1) + AceSize(2) + Mask(4) → SID 从第 8 字节开始。
            if unsafe { *base } != ACCESS_ALLOWED_ACE_TYPE {
                continue;
            }
            // 只继承给子项的 ACE 不作用于本对象，不算数。
            const INHERIT_ONLY_ACE: u8 = 0x08;
            if unsafe { *base.add(1) } & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            let mask = unsafe { std::ptr::read_unaligned(base.add(4) as *const u32) };
            if !rights_covered(mask, rights) {
                continue;
            }
            if unsafe { EqualSid(base.add(8) as PSID, sid) } != 0 {
                found = true;
                break;
            }
        }
    }
    unsafe {
        LocalFree(sd);
    }
    found
}

/// 系统 shell 的绝对路径（容器里显式给镜像路径比让系统搜索可靠）。
fn system_shell() -> PathBuf {
    if let Some(root) = std::env::var_os("SystemRoot") {
        let p = PathBuf::from(root).join("System32").join("cmd.exe");
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("cmd.exe")
}

/// 把自己放进一个 Job Object：最后一个句柄关闭（本进程消亡，含被杀）= 整棵树立即终止。
fn join_kill_on_close_job(max_processes: u32) -> Result<(), String> {
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

/// 把工具进程放进容器里跑：不给任何 capability（= 断网），stdio 用外层给的那三个句柄。
fn run_in_container(sid: PSID, spec: &FenceSpec, command: &str) -> Result<i32, String> {
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
        super::windows_program_separators(command)
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
            EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
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

/// 本机能不能强制住容器围栏（**不写任何目录 ACL**）：建容器 profile + 派生容器 SID 就是容器能起来的全部前提。
/// 三态：profile 建不起来（环境拒绝建）= 环境结论；profile 建得起来却派生不出 SID = 我们的步骤写错了。
pub fn verify(spec: &FenceSpec, _command: &str) -> FenceVerdict {
    let name = container_name(spec);
    if let Err(e) = ensure_profile(&name) {
        return FenceVerdict::EnvUnavailable(format!("容器 profile 建不起来：{}", e));
    }
    match container_sid(&name) {
        Ok(sid) => {
            free_sid(sid);
            FenceVerdict::Enforced
        }
        Err(e) => FenceVerdict::Broken(format!("容器 profile 已建起却派生不出容器 SID：{}", e)),
    }
}

pub fn run_fenced(spec: &FenceSpec, prepared: bool, home: Option<&Path>, command: &str) -> i32 {
    if let Err(e) = join_kill_on_close_job(MAX_PROCESSES) {
        eprintln!("[围栏] 进程树围栏安装失败：{}", e);
    }
    // 容器要先把读放行与落点做好（改本机目录 ACL）才可能真跑起来：外层没授权就直接按无围栏执行，
    // 不去试一个注定读不到模块目录与解释器的容器（那样只会把工具报成一堆"拒绝访问"）。
    if !prepared {
        eprintln!("[围栏] 外层未授权本机写入：容器围栏不可用，按如实降级继续执行");
        return run_unfenced(spec, command);
    }
    let name = container_name(spec);
    // 容器身份先立起来（profile 是容器能读到系统目录的前提）。
    if let Err(e) = ensure_profile(&name) {
        eprintln!("[围栏] {}{}）：按如实降级继续执行", ENV_BLOCKED_MARK, e);
        return run_unfenced(spec, command);
    }
    // 建过就记进台账：守门进程是唯一真正建 profile 的地方，外层只知道"该建"、不知道"建成了"。
    if let Some(h) = home {
        record_profile(h, &name);
    }
    let sid = match container_sid(&name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[围栏] 容器围栏未生效（{}）：按如实降级继续执行", e);
            return run_unfenced(spec, command);
        }
    };
    let outcome = run_in_container(sid, spec, command);
    free_sid(sid);
    match outcome {
        Ok(code) => code,
        Err(e) => {
            // 容器起不来也要如实说清，并退回普通方式执行（能力等级已在启动报告里说明）。
            eprintln!("[围栏] 容器围栏未生效（{}）：按如实降级继续执行", e);
            run_unfenced(spec, command)
        }
    }
}

/// 无围栏执行：容器不可用（外层没授权、profile 建不起来、容器起不来）时的如实降级——
/// 命令仍交系统 shell 解释、cwd 仍是模块根，只是少了容器那层强制（启动报告里已说明能力等级）。
fn run_unfenced(spec: &FenceSpec, command: &str) -> i32 {
    match shell_command(command).current_dir(&spec.cwd).status() {
        Ok(s) => s.code().unwrap_or(FENCE_FAILED),
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 容器 profile **一个 agent 一个**：同名 agent 跨会话复用同一个容器身份（数量有界），换 agent 就换 profile。
    #[test]
    fn container_profile_is_one_per_agent() {
        let a = FenceSpec {
            agent: "甲".to_string(),
            rw: vec![PathBuf::from("session").join("w1")],
            cwd: PathBuf::from("modules").join("m0"),
            ro: Vec::new(),
            net: false,
        };
        let mut b = a.clone();
        b.rw = vec![PathBuf::from("session").join("w2")];
        b.cwd = PathBuf::from("modules").join("m1");
        let mut c = a.clone();
        c.agent = "乙".to_string();
        assert_eq!(
            container_name(&a),
            container_name(&b),
            "同一个 agent 的不同会话共用一个 profile"
        );
        assert_ne!(container_name(&a), container_name(&c), "不同 agent 不共用");
        assert!(is_our_profile(&container_name(&a)));
    }

    /// 清扫只认自己的前缀：别人的容器 profile 一个都不许动。
    #[test]
    fn profile_sweep_only_matches_our_prefix() {
        assert!(
            is_our_profile("solomni.agent.0123456789abcdef"),
            "Windows 会把包目录名转小写"
        );
        assert!(is_our_profile("Solomni.Agent.0123456789abcdef"));
        assert!(!is_our_profile("microsoft.windows.notepad"));
        assert!(!is_our_profile("solomni"));
        assert!(!is_our_profile(""));
    }

    /// 已有 ACE 的权限位必须**覆盖得住**才算数：只看"SID 在场"会让基线被一个只有 SYNCHRONIZE 的继承 ACE
    /// 整条挡掉（真机上解释器目录就是这样，容器里连解释器都读不到）；通用位与展开后的具体位要等价看待。
    #[test]
    fn existing_ace_must_cover_the_rights_we_need() {
        assert!(
            !rights_covered(0x0010_0000 /* SYNCHRONIZE */, RIGHTS_RO),
            "只有 SYNCHRONIZE 不算覆盖"
        );
        assert!(
            !rights_covered(FILE_GENERIC_READ, RIGHTS_RO),
            "只有读不算覆盖读+执行"
        );
        assert!(
            rights_covered(FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, RIGHTS_RO),
            "读+执行刚够"
        );
        assert!(
            rights_covered(GENERIC_READ | GENERIC_EXECUTE, RIGHTS_RO),
            "通用位与具体位等价"
        );
        assert!(
            rights_covered(GENERIC_ALL, RIGHTS_RW),
            "GENERIC_ALL 覆盖一切"
        );
        assert!(
            !rights_covered(FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, RIGHTS_RW),
            "只读+执行不覆盖读写"
        );
        assert!(rights_covered(FILE_ALL_ACCESS, RIGHTS_RW));
    }

    /// 授权这条路的真机验收：真去改一个目录的 DACL。
    /// 本机环境不允许改 ACL 时（例如被沙箱挡住）如实打印原因并跳过——不静默当作通过。
    #[test]
    fn grants_are_written_when_the_environment_allows_it() {
        if !capability().fs {
            eprintln!(
                "[探针] 本机不允许改目录 ACL（{}）：授权探针跳过（不静默当作通过）——请在普通 shell 里重跑 cargo test 验证",
                capability().note
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("solomni-grant-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建探针目录");
        let spec = FenceSpec {
            agent: "probe".to_string(),
            rw: vec![dir.clone()],
            cwd: dir.clone(),
            ro: Vec::new(),
            net: false,
        };
        let prepared = Mutex::new(std::collections::BTreeSet::new());
        // 台账落在探针自己的临时目录里（不碰真实 .home/）。
        let home = dir.join("ledger");
        let outcome = prepare_fence(&spec, "cmd", &prepared, &home);
        assert!(outcome.is_ok(), "授权应当成功：{:?}", outcome.err());
        // 收尾必须把自己写下的权限项按台账撤掉：测试不在本机留痕（撤不动就报出来，不静默）。
        let report = clean(&home).expect("回收应当成功");
        assert!(report.contains("撤销"), "回收要如实报数量：{}", report);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 撤销的真效果：授权 → 撤权 → 目标目录上不再有该容器 SID 的 ACE。
    /// 与上一条一样只在能改 ACL 的环境里真跑（本机受限沙箱会如实跳过）。
    #[test]
    fn revoke_removes_the_container_ace_from_the_given_roots() {
        if !capability().fs {
            eprintln!(
                "[探针] 本机不允许改目录 ACL（{}）：撤销探针跳过（不静默当作通过）",
                capability().note
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("solomni-revoke-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建探针目录");
        let spec = FenceSpec {
            agent: "probe".to_string(),
            rw: vec![dir.clone()],
            cwd: dir.clone(),
            ro: Vec::new(),
            net: false,
        };
        let home = dir.join("ledger");
        let prepared = Mutex::new(std::collections::BTreeSet::new());
        prepare_fence(&spec, "cmd", &prepared, &home).expect("授权应当成功");
        let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
        assert!(
            has_ace_for(sid, &dir, RIGHTS_RW),
            "授权后根上应当有容器 SID 的 ACE"
        );
        free_sid(sid);
        release_fence_home(&spec, Some(&home)).expect("撤权应当成功");
        let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
        let still = has_ace_for(sid, &dir, RIGHTS_RW);
        free_sid(sid);
        assert!(!still, "撤权后根上不该再有该容器 SID 的 ACE");
        // 基线授权（解释器目录只读）也记在同一份台账里，一并按台账撤干净。
        clean(&home).expect("基线回收应当成功");
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// 只读档的真机验收：授权的只读根上写下的是**只读 ACE**，且撤权能把它撤净。
    /// 与读写授权分开断言——只读档的价值就在于"读得到、写不进"。
    #[test]
    fn read_only_grants_write_ro_aces_and_revoke_removes_them() {
        if !capability().fs {
            eprintln!(
                "[探针] 本机不允许改目录 ACL（{}）：只读授权探针跳过（不静默当作通过）",
                capability().note
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("solomni-ro-probe-{}", std::process::id()));
        let ro = std::env::temp_dir().join(format!("solomni-ro-target-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建探针目录");
        std::fs::create_dir_all(&ro).expect("建只读根");
        let spec = FenceSpec {
            agent: "probe-ro".to_string(),
            rw: vec![dir.clone()],
            ro: vec![ro.clone()],
            cwd: dir.clone(),
            net: false,
        };
        let home = dir.join("ledger");
        let prepared = Mutex::new(std::collections::BTreeSet::new());
        prepare_fence(&spec, "cmd", &prepared, &home).expect("授权应当成功");
        let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
        assert!(has_ace_for(sid, &ro, RIGHTS_RO), "只读根上要有只读 ACE");
        assert!(
            !has_ace_for(sid, &ro, RIGHTS_RW),
            "只读根上不该有读写 ACE——那正是只读档的意义"
        );
        free_sid(sid);
        release_fence_home(&spec, Some(&home)).expect("撤权应当成功");
        let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
        let still = has_ace_for(sid, &ro, RIGHTS_RO);
        free_sid(sid);
        assert!(!still, "撤权后只读根上不该再有该容器 SID 的 ACE");
        clean(&home).expect("台账回收应当成功");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&ro);
    }
}
