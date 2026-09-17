//! macOS 后端：seatbelt（sandbox_init）把「可达到哪些路径」变成内核强制。
//! sandbox_init 是私有且已弃用的 ABI（如实写在这里，不假装它是公开接口）：
//! 生成一份 profile 文本——默认拒绝一切，再逐条放行运行基线（只读）与 spec.rw（读写），
//! spec.net 为假时连网络一起拒。装不上时如实降级（打印说明后照常执行），启动时已报告能力等级。

use super::{shell_command, Capability, FENCE_FAILED};
use crate::core::fence::FenceSpec;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
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
    // 动态加载器的共享缓存（现代 macOS 上在这里）。
    "/private/var/db/dyld",
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
        Ok(s) => match s.code() {
            Some(c) => c,
            None => {
                // 被信号结束（例如内核按围栏规则直接杀）：如实说出信号，便于定位，
                // 否则外层只看到"命令未执行"，看不出是围栏干的还是程序自己崩的。
                use std::os::unix::process::ExitStatusExt;
                match s.signal() {
                    Some(sig) => eprintln!("[围栏] 工具进程被信号结束：signal {}", sig),
                    None => eprintln!("[围栏] 工具进程没有正常结束，也拿不到信号号"),
                }
                FENCE_FAILED
            }
        },
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
    // 规则规模如实报一行：失败时（探针/端到端日志）能据此判断"是不是规则太宽/太窄"。
    eprintln!(
        "[围栏] seatbelt 规则 {} 条（默认拒绝；放行只读 {} 处、读写 {} 处）",
        profile.lines().count().saturating_sub(5),
        spec.rw.len(),
        spec.rw.len()
    );
    // 排障开关：把解释器解析结果与完整 profile 打出来（探针打开它；正常运行不打，免得污染工具回执）。
    if std::env::var("SOLOMNI_FENCE_PROFILE").is_ok() {
        let dirs: Vec<String> = super::interpreter_dirs(command)
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        eprintln!("[围栏] 命令：{}", command);
        eprintln!("[围栏] 解析出的解释器目录：{:?}", dirs);
        eprintln!("[围栏] seatbelt profile：\n{}", profile);
    }
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
    // 取舍写在这里：`file-read-metadata` 全局放行（只 stat：存在性/大小/时间戳），
    // 因为**路径解析本身**就需要它——只给祖先目录放行不够（进程解析 /Users/... 时还要读中间符号链接项），
    // 少了它连 shell 都起不来（真机上表现为工具进程被信号 6 结束）。
    // 内容读取（file-read-data）仍然逐条放行，越界读照样拿不到内容。
    // 与路径无关的放行：进程/加载器起来所需的全部内核操作。只有 process* + sysctl-read + mach-lookup 时，
    // 加载器仍会 abort（真机上表现为工具进程被信号 6 结束、子进程一个字节都不输出）；
    // file-map-executable 必须**全局**放行——dyld 要把可执行文件与动态库 mmap 进内存，逐路径列举覆盖不全，
    // 凡没列到的映射都直接 abort。它不构成越界读：映射仍要先拿到该路径的 file-read-data，内容读取照旧逐条放行。
    let mut out = String::from(
        "(version 1)\n(deny default)\n(allow process*)\n(allow sysctl-read)\n(allow mach*)\n(allow ipc*)\n(allow signal)\n(allow system-socket)\n(allow system-fsctl)\n(allow system-info)\n(allow file-read-metadata)\n(allow file-read* (literal \"/\"))\n(allow file-map-executable)\n",
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
        // literal 而不是 subpath：穿过祖先只需要对**祖先本身**取元数据；
        // 用 subpath 会把整棵子树的元数据都放行，等于把围栏开成筛子。
        out.push_str(&format!("(allow file-read-metadata (literal \"{}\"))\n", escape(m)));
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

