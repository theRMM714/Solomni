//! Linux 后端：Landlock（内核 5.13+）把「可达到哪些路径」变成内核强制——无需 root、无需额外二进制。
//! 规则：spec.rw 的每个根读写与目录操作；运行基线（系统只读区）只读可执行；其余一律拒绝。
//! 机制不可用时（老内核、或规则被内核拒）如实降级：打印说明后照常执行——能力等级在启动时已如实告知，不静默假装有围栏。
//! 只做文件系统；网络在 spec.net 为假时靠调用方（虚拟机档）断网，本档不承诺。

use super::{shell_command, Capability, FENCE_FAILED};
use crate::core::fence::FenceSpec;
use std::ffi::CString;
use std::os::raw::c_int;
use std::os::unix::process::CommandExt;
use std::path::Path;

/// 稳定标记：本机 Landlock 机制有效（自检已过），但我们的规则/安装步骤装不上。
/// 与「本机内核不能围栏」是两回事——前者是代码写错（掩码/路径），探针据此响亮失败；
/// 后者才是环境结论，探针如实跳过。
pub const RULES_REJECTED_MARK: &str = "landlock 规则被拒绝";

/// 自检之后仍然装不上 = 规则写错，统一带上稳定标记（与 macOS 的 PROFILE_REJECTED_MARK 对称）。
fn rules_rejected(reason: String) -> String {
    format!("{}：{}", RULES_REJECTED_MARK, reason)
}

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

/// 目录专属位：规则挂在**普通文件/设备**上时，内核对这些位直接回 EINVAL。
/// （READ_ONLY_BASELINE 里的 /etc/ld.so.cache、/dev/null 等是文件，不能带着 FS_READ_DIR 去 add_rule。）
const DIR_ONLY: u64 = FS_READ_DIR
    | FS_REMOVE_DIR
    | FS_REMOVE_FILE
    | FS_MAKE_CHAR
    | FS_MAKE_DIR
    | FS_MAKE_REG
    | FS_MAKE_SOCK
    | FS_MAKE_FIFO
    | FS_MAKE_BLOCK
    | FS_MAKE_SYM
    | FS_REFER;

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

/// 自检：本机 Landlock 装上之后是否真的产生约束（结果缓存，只探一次）。
/// 只问 ABI 版本不够——landlock_add_rule 会因为规则细节被内核拒（例如掩码带了该类型不支持的位），
/// 那之后 landlock_restrict_self 根本没被调用，围栏等于不存在。
fn landlock_confines() -> bool {
    static EFFECTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EFFECTIVE.get_or_init(landlock_confines_probe)
}

/// 装一个「什么都不放行」的最小 ruleset，再去读金丝雀：读不到才算本机真能围。
/// 子进程退出码：0 = 读到了（没约束）、1 = 被挡住（机制有效）、2 = 装不上（无法判定）。
fn landlock_confines_probe() -> bool {
    let canary =
        std::env::temp_dir().join(format!("solomni-landlock-selfcheck-{}", std::process::id()));
    if std::fs::write(&canary, "canary").is_err() {
        return false;
    }
    let verdict = unsafe {
        let pid = libc::fork();
        if pid == 0 {
            let abi = match abi_version() {
                Ok(a) => a,
                Err(_) => libc::_exit(2),
            };
            let attr = RulesetAttr {
                handled_access_fs: mask_for(abi, RW_ALL | RO_ALL),
            };
            let fd = sys_create_ruleset(&attr, std::mem::size_of::<RulesetAttr>(), 0);
            if fd < 0 {
                libc::_exit(2);
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                libc::_exit(2);
            }
            if sys_restrict_self(fd, 0) != 0 {
                libc::_exit(2);
            }
            let read = std::fs::read_to_string(&canary)
                .map(|s| s.contains("canary"))
                .unwrap_or(false);
            libc::_exit(if read { 0 } else { 1 });
        }
        if pid < 0 {
            0
        } else {
            let mut status: c_int = 0;
            libc::waitpid(pid, &mut status, 0);
            if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 1 {
                1
            } else {
                0
            }
        }
    };
    let _ = std::fs::remove_file(&canary);
    verdict == 1
}

pub fn capability() -> Capability {
    let abi = abi_version();
    if abi.is_ok() && landlock_confines() {
        Capability {
            fs: true,
            net: false,
            tree: true,
            note: format!(
                "Linux：Landlock（ABI {}）强制文件系统可达范围；出站网络不围栏",
                abi.unwrap_or(-1)
            ),
        }
    } else {
        Capability {
            fs: false,
            net: false,
            tree: true,
            note: "Linux：本机 Landlock 不产生实际约束（内核不支持，或规则被内核拒绝）——文件系统围栏如实降级，只有进程树围栏"
                .to_string(),
        }
    }
}

pub fn run_fenced(spec: &FenceSpec, command: &str) -> i32 {
    match install(spec, command) {
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

fn install(spec: &FenceSpec, command: &str) -> Result<(), String> {
    // 先自检：本机 Landlock 是否真的产生约束。装上了也可能完全没约束，别假装有围栏。
    if !landlock_confines() {
        return Err("本机 Landlock 不产生实际约束（内核不支持，或规则被内核拒绝）".to_string());
    }
    // 自检之后的任何失败都是「我们的规则/安装步骤有问题」：带标记，探针据此响亮失败。
    let abi = abi_version().map_err(rules_rejected)?;
    let attr = RulesetAttr {
        handled_access_fs: mask_for(abi, RW_ALL | RO_ALL),
    };
    let fd = sys_create_ruleset(&attr, std::mem::size_of::<RulesetAttr>(), 0);
    if fd < 0 {
        return Err(rules_rejected(format!(
            "landlock_create_ruleset 失败：{}",
            std::io::Error::last_os_error()
        )));
    }
    let allowed_rw = mask_for(abi, RW_ALL);
    for root in &spec.rw {
        add_rule(fd, root, allowed_rw).map_err(rules_rejected)?;
    }
    if !spec.cwd.as_os_str().is_empty() {
        add_rule(fd, &spec.cwd, allowed_rw).map_err(rules_rejected)?;
    }
    for p in READ_ONLY_BASELINE {
        let path = Path::new(p);
        if path.exists() {
            add_rule(fd, path, RO_ALL).map_err(rules_rejected)?;
        }
    }
    // 命令里解释器的安装目录也要只读放行：否则解释器装在 /usr 之外（pyenv、homebrew、自装）时，工具在围栏里起不来。
    for dir in super::interpreter_dirs(command) {
        add_rule(fd, &dir, RO_ALL).map_err(rules_rejected)?;
    }
    // Landlock 的前置条件：不许再提权。
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(rules_rejected(format!(
            "prctl(PR_SET_NO_NEW_PRIVS) 失败：{}",
            std::io::Error::last_os_error()
        )));
    }
    if sys_restrict_self(fd, 0) != 0 {
        return Err(rules_rejected(format!(
            "landlock_restrict_self 失败：{}",
            std::io::Error::last_os_error()
        )));
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
    // 掩码必须跟根的类型对得上：内核对非目录 parent 会拒掉目录专属位，回 EINVAL。
    // 少了这个分叉，规则链会在第一个「文件型基线路径」（/etc/ld.so.cache）上断掉，
    // 后面连 landlock_restrict_self 都走不到——围栏一条都不生效。
    // 用 metadata 判类型（与上面 open(O_PATH) 一样跟随符号链接，/etc/localtime 这类别名因此判得准）。
    let allowed_access = match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => access,
        Ok(_) => access & !DIR_ONLY,
        Err(e) => {
            unsafe {
                libc::close(parent);
            }
            return Err(format!("取允许的根的类型失败（{}）：{}", path.display(), e));
        }
    };
    let rule = PathBeneathAttr {
        allowed_access,
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

