//! macOS 后端：seatbelt（sandbox_init）把「可达到哪些路径」变成内核强制。
//! sandbox_init 是私有且已弃用的 ABI（如实写在这里，不假装它是公开接口）：
//! 生成一份 profile 文本——默认拒绝一切，再逐条放行运行基线（只读）与 spec.rw（读写），
//! spec.net 为假时连网络一起拒。装不上时如实降级（打印说明后照常执行），启动时已报告能力等级。

use super::{shell_command, Capability, FENCE_FAILED};
use crate::core::fence::FenceSpec;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::os::unix::process::CommandExt;
use std::path::Path;

extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// 只读运行基线：解释器与系统运行库所在处（脚本要跑起来必须读得到这些）。
const READ_ONLY_BASELINE: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library/Apple",
    "/private/etc",
    "/private/var/db/timezone",
    "/dev/null",
    "/dev/zero",
    "/dev/urandom",
    "/dev/random",
    "/private/tmp",
];

pub fn capability() -> Capability {
    if seatbelt_confines() {
        Capability {
            fs: true,
            net: true,
            tree: true,
            note: "macOS：seatbelt（sandbox_init，私有且已弃用的 ABI）强制可达范围并按档位断网"
                .to_string(),
        }
    } else {
        Capability {
            fs: false,
            net: false,
            tree: true,
            note: "macOS：本机 sandbox_init 不产生实际约束（该私有 ABI 在新版 macOS 上已失效）——文件系统与断网围栏如实降级，只有进程树围栏"
                .to_string(),
        }
    }
}

/// 自检：最小 profile `(deny default)` 装进子进程后，读金丝雀文件必须失败。
/// 这是**唯一**能分辨「机制在本机失效」与「我们的 profile 写错」的办法：
/// 前者如实降级，后者由探针响亮失败（见 tests/macos/probes/fence.rs）。
fn seatbelt_confines() -> bool {
    static EFFECTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EFFECTIVE.get_or_init(seatbelt_confines_probe)
}

fn seatbelt_confines_probe() -> bool {
    let canary = std::env::temp_dir().join(format!("solomni-seatbelt-selfcheck-{}", std::process::id()));
    if std::fs::write(&canary, "canary").is_err() {
        return false;
    }
    let path = CString::new(canary.to_string_lossy().as_bytes()).expect("临时路径不含 NUL");
    let profile = CString::new("(version 1)(deny default)").expect("字面量不含 NUL");
    // 子进程退出码：0 = 读到了（没约束）、1 = 被挡住（机制有效）、2 = 装不上（无法判定）
    let verdict = unsafe {
        let pid = libc::fork();
        if pid == 0 {
            let mut err: *mut c_char = std::ptr::null_mut();
            let rc = sandbox_init(profile.as_ptr(), 0, &mut err);
            if rc != 0 {
                libc::_exit(2);
            }
            let read = std::fs::read_to_string(path.to_string_lossy().as_ref())
                .map(|s| s.contains("canary"))
                .unwrap_or(false);
            libc::_exit(if read { 0 } else { 1 });
        }
        let mut status: c_int = 0;
        libc::waitpid(pid, &mut status, 0);
        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 1 {
            1
        } else {
            0
        }
    };
    let _ = std::fs::remove_file(&canary);
    verdict == 1
}

pub fn run_fenced(spec: &FenceSpec, command: &str) -> i32 {
    match install(spec, command) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[围栏] 文件系统围栏未生效（{}）：按如实降级继续执行", e);
        }
    }
    let mut cmd = shell_command(command);
    cmd.current_dir(&spec.cwd);
    // 孤儿防护：**不要**给工具另起进程组。守门进程自成一组（由拉起它的一侧设置），
    // 杀那一组 = 连根杀整棵工具树——这与 Windows 的 Job Object 是同一套语义；
    // 另起组会让工具逃出那一组，外部杀掉守门进程后它就变成孤儿（探针会抓住这种行为）。
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(FENCE_FAILED),
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}

fn install(spec: &FenceSpec, command: &str) -> Result<(), String> {
    // 先自检：本机 sandbox_init 是否真的产生约束。不产生就别假装装上了（探针据此如实跳过）。
    if !seatbelt_confines() {
        return Err("本机 sandbox_init 不产生实际约束（该私有 ABI 在新版 macOS 上已失效）".to_string());
    }
    let profile = profile_text(spec, command);
    let c = CString::new(profile).map_err(|_| "profile 文本含非法字节".to_string())?;
    let mut errbuf: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { sandbox_init(c.as_ptr(), 0, &mut errbuf) };
    if rc != 0 {
        let why = if errbuf.is_null() {
            "sandbox_init 失败（未给出原因）".to_string()
        } else {
            let s = unsafe { CStr::from_ptr(errbuf) }
                .to_string_lossy()
                .into_owned();
            unsafe { sandbox_free_error(errbuf) };
            s
        };
        return Err(why);
    }
    Ok(())
}

/// 生成 seatbelt profile：默认拒绝，再逐条放行（路径按 seatbelt 的字符串转义）。
/// 只读范围 = 系统只读基线 + **命令里解释器的安装目录**（否则工具在围栏里起不来）；
/// 另外给所有被放行路径的祖先目录放行"只读元数据"（路径解析要能按名穿过）。
fn profile_text(spec: &FenceSpec, command: &str) -> String {
    let mut out = String::from(
        "(version 1)\n(deny default)\n(allow process*)\n(allow sysctl-read)\n(allow mach-lookup)\n",
    );
    let mut ro_paths: Vec<String> = Vec::new();
    for p in READ_ONLY_BASELINE {
        let path = Path::new(p);
        if path.exists() {
            ro_paths.push(p.to_string());
        }
    }
    for dir in super::interpreter_dirs(command) {
        ro_paths.push(dir.to_string_lossy().replace('\\', "/"));
    }
    ro_paths.sort();
    ro_paths.dedup();
    for p in &ro_paths {
        out.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", escape(p)));
    }
    let mut rw_paths: Vec<String> = Vec::new();
    for root in &spec.rw {
        let path = root.to_string_lossy().replace('\\', "/");
        rw_paths.push(path.clone());
        out.push_str(&format!(
            "(allow file-read* file-write* (subpath \"{}\"))\n",
            escape(&path)
        ));
    }
    // 祖先目录只放行"读元数据"：路径解析要能按名穿过它们（与 Windows 的 FILE_TRAVERSE 对称），
    // 但不能读内容——少了这条，被放行目录里的命令行都跑不起来（连路径都解析不了）。
    let mut metas: Vec<String> = Vec::new();
    for p in ro_paths.iter().cloned().chain(rw_paths.into_iter()) {
        let mut cur = p.as_str();
        while let Some(i) = cur.rfind('/') {
            if i == 0 {
                break;
            }
            cur = &cur[..i];
            metas.push(cur.to_string());
        }
    }
    metas.sort();
    metas.dedup();
    for m in &metas {
        out.push_str(&format!("(allow file-read-metadata (subpath \"{}\"))\n", escape(m)));
    }
    if !spec.net {
        out.push_str("(deny network*)\n");
    }
    out
}

/// seatbelt profile 里的字符串转义：反斜杠与引号。
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

