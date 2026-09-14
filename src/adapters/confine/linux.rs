//! Linux 后端：Landlock（内核 5.13+）把「可达到哪些路径」变成内核强制——无需 root、无需额外二进制。
//! 规则：spec.rw 的每个根读写与目录操作；运行基线（系统只读区）只读可执行；其余一律拒绝。
//! 机制不可用时（老内核）如实降级：打印说明后照常执行——能力等级在启动时已如实告知，不静默假装有围栏。
//! 只做文件系统；网络在 spec.net 为假时靠调用方（虚拟机档）断网，本档不承诺。

use super::{shell_command, Capability, FENCE_FAILED};
use crate::core::fence::FenceSpec;
use std::ffi::CString;
use std::os::unix::process::CommandExt;
use std::path::Path;

/// 只读运行基线：解释器与系统运行库所在处（脚本要跑起来必须读得到这些）。
const READ_ONLY_BASELINE: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/lib32",
    "/etc/ld.so.cache",
    "/etc/localtime",
    "/etc/terminfo",
    "/dev/null",
    "/dev/zero",
    "/dev/urandom",
    "/dev/random",
    "/proc/self",
    "/proc/meminfo",
    "/sys/devices/system/cpu",
];

const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
/// ABI 2（内核 5.19）起才有"跨目录改名/链接"这一位。
const FS_REFER: u64 = 1 << 13;
/// ABI 3（内核 6.2）起才有 truncate 这一位。
const FS_TRUNCATE: u64 = 1 << 14;

const RW_ALL: u64 = FS_EXECUTE
    | FS_WRITE_FILE
    | FS_READ_FILE
    | FS_READ_DIR
    | FS_REMOVE_DIR
    | FS_REMOVE_FILE
    | FS_MAKE_CHAR
    | FS_MAKE_DIR
    | FS_MAKE_REG
    | FS_MAKE_SOCK
    | FS_MAKE_FIFO
    | FS_MAKE_BLOCK
    | FS_MAKE_SYM
    | FS_REFER
    | FS_TRUNCATE;
const RO_ALL: u64 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

fn sys_create_ruleset(attr: *const RulesetAttr, size: usize, flags: u32) -> i32 {
    unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, attr, size, flags) as i32 }
}

fn sys_add_rule(ruleset_fd: i32, rule_type: u32, attr: *const PathBeneathAttr, flags: u32) -> i32 {
    unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset_fd,
            rule_type,
            attr,
            flags,
        ) as i32
    }
}

fn sys_restrict_self(ruleset_fd: i32, flags: u32) -> i32 {
    unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, flags) as i32 }
}

/// 本机 Landlock ABI 版本（拿不到 = 内核不支持）。
fn abi_version() -> Result<i32, String> {
    let v = sys_create_ruleset(std::ptr::null(), 0, LANDLOCK_CREATE_RULESET_VERSION);
    if v < 0 {
        return Err(format!("{}", std::io::Error::last_os_error()));
    }
    Ok(v)
}

/// 按 ABI 去掉本内核不认识的那些位（否则 add_rule 会 EINVAL）。
fn mask_for(abi: i32, mut access: u64) -> u64 {
    if abi < 2 {
        access &= !FS_REFER;
    }
    if abi < 3 {
        access &= !FS_TRUNCATE;
    }
    access
}

pub fn capability() -> Capability {
    match abi_version() {
        Ok(abi) => Capability {
            fs: true,
            net: false,
            tree: true,
            note: format!(
                "Linux：Landlock（ABI {}）强制文件系统可达范围；出站网络不围栏",
                abi
            ),
        },
        Err(e) => Capability {
            fs: false,
            net: false,
            tree: true,
            note: format!(
                "Linux：本机内核不支持 Landlock（{}）——工具进程没有文件系统围栏，如实降级",
                e
            ),
        },
    }
}

pub fn run_fenced(spec: &FenceSpec, command: &str) -> i32 {
    match install(spec) {
        Ok(()) => {}
        Err(e) => {
            // 如实降级：机制装不上就不装，但不假装装上了（启动时已报告能力等级）。
            eprintln!("[围栏] 文件系统围栏未生效（{}）：按如实降级继续执行", e);
        }
    }
    let mut cmd = shell_command(command);
    cmd.current_dir(&spec.cwd);
    // 父进程（守门进程）一死，工具进程跟着死——不留孤儿。
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            );
            Ok(())
        });
    }
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(FENCE_FAILED),
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}

fn install(spec: &FenceSpec) -> Result<(), String> {
    let abi = abi_version()?;
    let attr = RulesetAttr {
        handled_access_fs: mask_for(abi, RW_ALL | RO_ALL),
    };
    let fd = sys_create_ruleset(&attr, std::mem::size_of::<RulesetAttr>(), 0);
    if fd < 0 {
        return Err(format!(
            "landlock_create_ruleset 失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    let allowed_rw = mask_for(abi, RW_ALL);
    for root in &spec.rw {
        add_rule(fd, root, allowed_rw)?;
    }
    if !spec.cwd.as_os_str().is_empty() {
        add_rule(fd, &spec.cwd, allowed_rw)?;
    }
    for p in READ_ONLY_BASELINE {
        let path = Path::new(p);
        if path.exists() {
            add_rule(fd, path, RO_ALL)?;
        }
    }
    // Landlock 的前置条件：不许再提权。
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "prctl(PR_SET_NO_NEW_PRIVS) 失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    if sys_restrict_self(fd, 0) != 0 {
        return Err(format!(
            "landlock_restrict_self 失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        libc::close(fd);
    }
    Ok(())
}

fn add_rule(ruleset_fd: i32, path: &Path, access: u64) -> Result<(), String> {
    let c = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| format!("路径含非法字节：{}", path.display()))?;
    let parent = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if parent < 0 {
        return Err(format!(
            "打不开允许的根（{}）：{}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let rule = PathBeneathAttr {
        allowed_access: access,
        parent_fd: parent,
    };
    let rc = sys_add_rule(ruleset_fd, LANDLOCK_RULE_PATH_BENEATH, &rule, 0);
    unsafe {
        libc::close(parent);
    }
    if rc != 0 {
        return Err(format!(
            "landlock_add_rule 失败（{}）：{}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

