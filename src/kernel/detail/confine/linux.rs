//! 目的：Linux 后端——Landlock（内核 5.13+）把「可达到哪些路径」变成内核强制，seccomp 把「能不能建网络 socket」变成内核强制；
//!   两者都不需要 root、不需要额外二进制。
//! 管：规则——spec.rw 的每个根读写与目录操作；运行基线（系统只读区）只读可执行；其余一律拒绝。
//!   断网——`net=false` 时用 seccomp 拒绝非 AF_UNIX / AF_NETLINK socket（TCP / UDP / raw 都出不去），
//!   `net=true` 不装网络过滤器（Linux 没有与 Windows / macOS 对等的入站限制，平台差异见 docs/kernel/README.md）。
//! 不管：虚拟机档网络——由 QEMU 网卡决定。
//! 联动：机制不可用时（老内核、规则被内核拒、seccomp 不可用）如实降级并打印说明；能力等级在启动报告里已如实告知。

use super::{shell_command, Capability, FenceVerdict, FENCE_FAILED};
use crate::kernel::api::FenceSpec;
use std::ffi::CString;
use std::os::raw::c_int;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// 目的：稳定标记：本机 Landlock 机制有效（自检已过），但我们的规则/安装步骤装不上。
///   与「本机内核不能围栏」是两回事——前者是代码写错（掩码/路径），探针据此响亮失败；
///   后者才是环境结论，探针如实跳过。
pub const RULES_REJECTED_MARK: &str = "landlock 规则被拒绝";

/// 目的：稳定标记：seccomp 自检有效，但网络过滤器的安装步骤装不上（规则写错）。
///   与「本机环境不允许 seccomp」是两回事——探针据此响亮失败，而不是当成环境跳过。
pub const SECCOMP_REJECTED_MARK: &str = "seccomp 规则被拒绝";

/// 目的：网络围栏未生效时的稳定说法（环境不允许 / 降级）；探针与日志都按它判定。
pub const NETWORK_NOT_ENFORCED: &str = "网络围栏未生效";

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
    // Node 起进程时初始化 OpenSSL 会读系统配置（python / C++ 不读）：不放行它，
    // JS 模块工具连 `node -e` 都起不来。两个常见落点各守一处，不存在的由 exists() 跳过。
    "/etc/ssl/openssl.cnf",
    "/etc/pki/tls/openssl.cnf",
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

// ---- seccomp：net=false 时阻断网络 socket ----
/// seccomp 的常量与 Linux 的 UAPI 对齐；libc 只给了结构体，动作位自己写，避免依赖额外的 seccomp crate。
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JEQ_K: u16 = 0x15;
const BPF_JSET_K: u16 = 0x45;
const BPF_RET_K: u16 = 0x06;
/// x32 ABI 的 syscall 号带这一位；不单独拦，x32 进程就能绕过下面的 socket 判断。
const X32_SYSCALL_BIT: u32 = 0x4000_0000;

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xC000_003E;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xC000_00B7;

/// 本架构的 AUDIT_ARCH；其它架构如实报不可用（seccomp 过滤器需要按 arch 校验）。
fn audit_arch() -> Option<u32> {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        Some(AUDIT_ARCH)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        None
    }
}

fn bpf_stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

fn bpf_jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

fn bpf_ret(action: u32) -> libc::sock_filter {
    bpf_stmt(BPF_RET_K, action)
}

/// 目的：net=false 的 seccomp 过滤器——只放行 AF_UNIX / AF_NETLINK 的 socket，其余域一律 EPERM；
///   同时拒绝 io_uring_setup（io_uring 里发的 socket 请求不受 seccomp 逐条约束）。
/// 约束：arch 不匹配直接杀进程（过滤器只服务本架构）；x32 syscall 一律拒绝。
fn network_filter(arch: u32) -> Vec<libc::sock_filter> {
    let deny = SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xffff);
    let socket = libc::SYS_socket as u32;
    let io_uring_setup = libc::SYS_io_uring_setup as u32;
    let args0 = std::mem::offset_of!(libc::seccomp_data, args) as u32;
    vec![
        // arch 校验
        bpf_stmt(
            BPF_LD_W_ABS,
            std::mem::offset_of!(libc::seccomp_data, arch) as u32,
        ),
        bpf_jump(BPF_JEQ_K, arch, 1, 0),
        bpf_ret(SECCOMP_RET_KILL_PROCESS),
        // syscall 号；x32 号段先拒
        bpf_stmt(
            BPF_LD_W_ABS,
            std::mem::offset_of!(libc::seccomp_data, nr) as u32,
        ),
        bpf_jump(BPF_JSET_K, X32_SYSCALL_BIT, 0, 1),
        bpf_ret(deny),
        // socket(2) 才看 domain；其它 syscall 跳过整个 socket 判断，直接到 io_uring 检查
        bpf_jump(BPF_JEQ_K, socket, 0, 4),
        bpf_stmt(BPF_LD_W_ABS, args0),
        bpf_jump(BPF_JEQ_K, libc::AF_UNIX as u32, 4, 0),
        bpf_jump(BPF_JEQ_K, libc::AF_NETLINK as u32, 3, 0),
        // 非 AF_UNIX / AF_NETLINK 的 socket：拒绝
        bpf_ret(deny),
        // 非 socket：只额外拦 io_uring_setup；其余一律放行
        bpf_jump(BPF_JEQ_K, io_uring_setup, 0, 1),
        bpf_ret(deny),
        bpf_ret(SECCOMP_RET_ALLOW),
    ]
}

/// 目的：装 seccomp 过滤器（调用方先做自检）。走到这里才失败 = 规则/安装步骤写错，带稳定标记。
fn install_network_filter() -> Result<(), String> {
    let Some(arch) = audit_arch() else {
        return Err(format!(
            "{}：本架构没有 seccomp 网络过滤器",
            SECCOMP_REJECTED_MARK
        ));
    };
    let mut filter = network_filter(arch);
    let prog = libc::sock_fprog {
        len: filter.len() as libc::c_ushort,
        filter: filter.as_mut_ptr(),
    };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "{}：prctl(PR_SET_NO_NEW_PRIVS) 失败：{}",
            SECCOMP_REJECTED_MARK,
            std::io::Error::last_os_error()
        ));
    }
    if unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER as libc::c_ulong,
            &prog as *const libc::sock_fprog,
        )
    } != 0
    {
        return Err(format!(
            "{}：prctl(PR_SET_SECCOMP) 失败：{}",
            SECCOMP_REJECTED_MARK,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// 目的：seccomp 自检——确认本机既能建 AF_INET socket，又能在过滤器装上后挡住它，且 AF_UNIX 仍可用。
///   只看「装上了没有」不够：外层可能已经有过滤器挡住 socket（此时 allow 模式也无法恢复），
///   或我们的规则把 AF_UNIX 也一起砍了。
fn network_confines() -> bool {
    if super::selfcheck_forced_unavailable() {
        return false;
    }
    static EFFECTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EFFECTIVE.get_or_init(network_confines_probe)
}

/// 子进程退出码：1 = 过滤器有效；0 = 基线不允许 / 装不上 / 过滤器没挡住或误伤 AF_UNIX。
fn network_confines_probe() -> bool {
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // fork 之后只做系统调用：失败一律 _exit(0)（不能执行 Rust 的析构/分配）。
        unsafe {
            let baseline = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            if baseline < 0 {
                libc::_exit(0);
            }
            libc::close(baseline);
            if install_network_filter().is_err() {
                libc::_exit(0);
            }
            let denied = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            if denied >= 0 {
                libc::close(denied);
                libc::_exit(0);
            }
            let unix = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
            if unix < 0 {
                libc::_exit(0);
            }
            libc::close(unix);
            libc::_exit(1);
        }
    }
    if pid < 0 {
        return false;
    }
    let mut status: c_int = 0;
    unsafe {
        libc::waitpid(pid, &mut status, 0);
    }
    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 1
}

/// 目的：net=false 时安装网络围栏；自检不过 = 本机环境结论，不是规则写错。
fn install_net() -> Result<(), String> {
    if !network_confines() {
        return Err("本机 seccomp 不产生实际约束（内核不支持或环境不允许）".to_string());
    }
    install_network_filter()
}

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
    // 测试专用的注入：探针要能确定性地走「本机不允许」这一路（见 confine::SELFCHECK_FAIL_FLAG）。
    if super::selfcheck_forced_unavailable() {
        return false;
    }
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
    let abi = abi_version().ok();
    let fs = abi.is_some() && landlock_confines();
    let net = network_confines();
    let note = match (fs, net) {
        (true, true) => format!(
            "Linux：Landlock（ABI {}）强制文件系统可达范围；seccomp 强制断网（net=false 时拒绝非本地 socket）",
            abi.unwrap_or(-1)
        ),
        (true, false) => format!(
            "Linux：Landlock（ABI {}）强制文件系统可达范围；断网机制不可用",
            abi.unwrap_or(-1)
        ),
        (false, true) => {
            "Linux：本机 Landlock 不产生实际约束——文件系统围栏如实降级；seccomp 断网可用".to_string()
        }
        (false, false) => {
            "Linux：本机文件系统围栏与断网机制都不可用——只有进程树围栏".to_string()
        }
    };
    Capability {
        fs,
        net,
        tree: true,
        note,
    }
}

/// 目的：`_prepared`（外层是否已完成本机授权）与 `_home`（容器 profile 的台账落点）只有 Windows 的容器围栏用得上：
///   Linux 的 Landlock 在守门进程里自足，也没有容器 profile 这一步。
pub fn run_fenced(
    spec: &FenceSpec,
    _prepared: bool,
    _home: Option<&std::path::Path>,
    command: &str,
    policy: &crate::kernel::api::FenceRunPolicy,
) -> i32 {
    // 文件系统与网络**独立**收口：一项装不上不该把另一项也丢掉。
    let fs = install_filesystem(spec, command);
    let net = if spec.net { Ok(()) } else { install_net() };
    let fs_enforced = fs.is_ok();
    let net_enforced = spec.net || net.is_ok();
    let fs_error = fs.as_ref().err();
    let net_error = net.as_ref().err();
    let required =
        (policy.require_fs && fs_error.is_some()) || (policy.require_net && net_error.is_some());
    if required && !policy.allow_degraded {
        if let Some(e) = fs_error {
            eprintln!("[围栏] 文件系统围栏未生效（{}）", e);
        }
        if let Some(e) = net_error {
            eprintln!("[围栏] {}（{}）", NETWORK_NOT_ENFORCED, e);
        }
        eprintln!("[围栏] 按全局策略拒绝执行");
        return FENCE_FAILED;
    }
    // 如实降级：机制装不上就不装，但不假装装上了（启动时已报告能力等级）。
    if let Some(e) = fs_error {
        eprintln!("[围栏] 文件系统围栏未生效（{}）：按如实降级继续执行", e);
    }
    if let Some(e) = net_error {
        eprintln!(
            "[围栏] {}（{}）：按如实降级继续执行",
            NETWORK_NOT_ENFORCED, e
        );
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
        Ok(s) => {
            let code = s.code().unwrap_or(FENCE_FAILED);
            super::note_fence_boundary(spec, fs_enforced, net_enforced, code);
            code
        }
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}

/// 目的：装围栏并**如实分类**结果：自检不过 = 本机环境结论（降级照跑）；自检过了还装不上 = 我们写错了。
///   探针早就按这两类分别处理（前者如实跳过、后者响亮失败），运行期在未授权时段也照这一份结论走。
pub fn verify(spec: &FenceSpec, command: &str) -> FenceVerdict {
    if !landlock_confines() {
        return FenceVerdict::EnvUnavailable(
            "本机 Landlock 不产生实际约束（内核不支持，或规则被内核拒绝）".to_string(),
        );
    }
    if !spec.net && !network_confines() {
        return FenceVerdict::EnvUnavailable(
            "本机 seccomp 不产生实际约束（内核不支持或环境不允许）".to_string(),
        );
    }
    // 两条路都试一遍：自检说机制有效却装不上 = 我们写错了，如实归到 Broken。
    let fs = install_rules(spec, command);
    let net = if spec.net { Ok(()) } else { install_net() };
    match (fs, net) {
        (Ok(()), Ok(())) => FenceVerdict::Enforced,
        (Err(e), _) => FenceVerdict::Broken(e),
        (_, Err(e)) => FenceVerdict::Broken(e),
    }
}

/// 守门进程路径：装文件系统围栏（自检不过与规则装不上都如实降级照跑——能力等级已在启动报告里说过）。
fn install_filesystem(spec: &FenceSpec, command: &str) -> Result<(), String> {
    if !landlock_confines() {
        return Err("本机 Landlock 不产生实际约束（内核不支持，或规则被内核拒绝）".to_string());
    }
    install_rules(spec, command)
}

/// 装规则（自检由调用方先做）：走到这里才失败 = 规则/安装步骤写错，一律带稳定标记。
fn install_rules(spec: &FenceSpec, command: &str) -> Result<(), String> {
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
    // **先收集所有放行规则，最后统一挂**：挂上第一条之后进程自己也被约束，
    // 后续 add_rule 要 open 的路径可能已打不开（会被误报成「规则被拒绝」）；先算清楚再装。
    let mut wanted: Vec<(PathBuf, u64)> = Vec::new();
    let allowed_rw = mask_for(abi, RW_ALL);
    for root in &spec.rw {
        wanted.push((root.clone(), allowed_rw));
    }
    // 工作目录（模块根）：工具进程要能在里面起（读 + 执行），但**不因此获得写**——
    // 模块目录默认只读时，写权只能来自 rw（module_write 授权）。
    if !spec.cwd.as_os_str().is_empty() {
        wanted.push((spec.cwd.clone(), RO_ALL));
    }
    // 用户显式授权的主机只读根（`sandbox.policy.host_read_roots`）：只给只读位，一个写位都不给。
    for root in &spec.ro {
        if !root.as_os_str().is_empty() {
            wanted.push((root.clone(), RO_ALL));
        }
    }
    // 只读子树（模块目录默认只读）：同为只读；与 ro 的分野只在 Windows 的递归位上。
    for root in &spec.ro_tree {
        if !root.as_os_str().is_empty() {
            wanted.push((root.clone(), RO_ALL));
        }
    }
    for p in READ_ONLY_BASELINE {
        let path = Path::new(p);
        if path.exists() {
            wanted.push((path.to_path_buf(), RO_ALL));
        }
    }
    // 命令里解释器的安装目录也要只读放行：否则解释器装在 /usr 之外（pyenv、homebrew、自装）时，工具在围栏里起不来。
    for dir in super::interpreter_dirs(command) {
        wanted.push((dir, RO_ALL));
    }
    for (path, access) in &wanted {
        add_rule(fd, path, *access).map_err(rules_rejected)?;
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
    // 掩码必须跟根的类型对得上（内核对非目录 parent 会拒目录专属位，回 EINVAL）：
    // 少了这个分叉，规则链会在第一个「文件型基线路径」（/etc/ld.so.cache）上断掉，围栏一条都不生效。
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
