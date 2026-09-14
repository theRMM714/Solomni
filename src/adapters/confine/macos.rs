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
    Capability {
        fs: true,
        net: true,
        tree: true,
        note: "macOS：seatbelt（sandbox_init，私有且已弃用的 ABI）强制可达范围并按档位断网"
            .to_string(),
    }
}

pub fn run_fenced(spec: &FenceSpec, command: &str) -> i32 {
    match install(spec) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[围栏] 文件系统围栏未生效（{}）：按如实降级继续执行", e);
        }
    }
    let mut cmd = shell_command(command);
    cmd.current_dir(&spec.cwd);
    // 孤儿防护：macOS 没有 Linux 的 PR_SET_PDEATHSIG，改为独立进程组——
    // 外层在超时/退出时按负 pid 杀整组（与运行期 kill_tree 同一套语义）。
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
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
    let profile = profile_text(spec);
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
fn profile_text(spec: &FenceSpec) -> String {
    let mut out = String::from(
        "(version 1)\n(deny default)\n(allow process*)\n(allow sysctl-read)\n(allow mach-lookup)\n",
    );
    for p in READ_ONLY_BASELINE {
        let path = Path::new(p);
        if path.exists() {
            out.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", escape(p)));
        }
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
    for p in READ_ONLY_BASELINE
        .iter()
        .map(|s| s.to_string())
        .chain(rw_paths.into_iter())
    {
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

