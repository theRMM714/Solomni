//! 守门进程：把围栏装进工具进程，然后才跑模块声明的命令（实现 core/fence.rs 的策略）。
//! 机制边界：本层只做机制——按平台把围栏（可读可写的根、断网、进程树围栏、资源上限）装好，
//! 策略（哪些根可达、放不放网）由 core 派生后经命令行传入。
//! 平台实现分文件：linux.rs（Landlock）/ macos.rs（seatbelt）/ windows.rs（Job Object + 容器）/ other.rs（如实降级）。
//! 能力不足时如实上报（capability），降级而非崩溃——绝不静默假装有围栏。

use crate::core::fence::FenceSpec;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod other;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use other as backend;

/// 守门模式参数：主程序带它启动 = 以守门进程身份执行（内部协议，用户不直接用）。
pub const FENCE_FLAG: &str = "--fence-run";
/// 围栏装不上时守门进程的退出码（工具执行据此如实报错，不静默）。
pub const FENCE_FAILED: i32 = 111;

/// 围栏授权释放的适配器（实现 core 的 FenceHost 端口）：core 只说「这个会话的围栏撤掉」。
pub struct FenceHostAdapter;

impl crate::core::ports::FenceHost for FenceHostAdapter {
    fn release(&self, spec: &FenceSpec) -> Result<(), String> {
        release_fence(spec)
    }
}

/// 本机能强制的围栏等级（如实上报给用户与日志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// 文件系统可达范围是否被真正强制。
    pub fs: bool,
    /// 出站网络是否被真正强制关闭。
    pub net: bool,
    /// 进程树是否连根围住（超时/退出能杀整棵）。
    pub tree: bool,
    /// 如实说明（机制名 + 限制）。
    pub note: String,
}

/// 本机能力（装配期如实告知）。
pub fn capability() -> Capability {
    backend::capability()
}

/// 组装守门进程的命令行：工具命令作为**数据**传递（不拼进 shell 字符串，杜绝注入）。
pub fn launcher(exe: &Path, spec: &FenceSpec, command: &str) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg(FENCE_FLAG).arg(spec.to_json()).arg("--").arg(command);
    cmd
}

/// 守门进程内：装围栏 → 跑命令 → 返回退出码。失败必须报错（stderr）并用 FENCE_FAILED 退出。
pub fn run_fenced(spec: &FenceSpec, command: &str) -> i32 {
    backend::run_fenced(spec, command)
}

/// 外层进程调用：把围栏要用的授权一次性做好（Windows 需要写目录 ACL；其它平台是空操作）。
/// prepared 是"已经授权过"的台账，避免每次工具调用重复改 ACL。
pub fn prepare_fence(
    spec: &FenceSpec,
    command: &str,
    prepared: &std::sync::Mutex<std::collections::BTreeSet<String>>,
    home: &std::path::Path,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::prepare_fence(spec, command, prepared, home)
    }
    #[cfg(not(windows))]
    {
        let _ = (spec, command, prepared, home);
        Ok(())
    }
}

/// 精确回收：按台账撤掉我们写过的权限项、删掉我们建过的容器 profile（`--fence-clean` 用）。
pub fn clean(home: &std::path::Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::clean(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Ok("本平台的围栏不留权限项，无需清理".to_string())
    }
}

/// 撤销一次会话的围栏授权（会话删除时由核心经 FenceHost 端口请求；其它平台是空操作）。
pub fn release_fence(spec: &FenceSpec) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::release_fence(spec)
    }
    #[cfg(not(windows))]
    {
        let _ = spec;
        Ok(())
    }
}

/// 命令里可能出现的外部程序：按 PATH 解析出真实路径（解析不出的跳过，不猜）。
/// 它们的**安装目录**必须放行（只读+执行），否则受限进程连解释器都起不来——Windows 的目录 ACL 与 macOS 的 seatbelt 都靠它。
pub(crate) fn interpreter_dirs(command: &str) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut candidates: Vec<String> = Vec::new();
    for raw in command.split([' ', '\t', '&', '|', ';', '\n']) {
        let token = raw.trim_matches(|c| c == '"' || c == '\'' || c == '(' || c == ')');
        if token.is_empty() || token.starts_with('-') || token.starts_with('/') || token.starts_with('%') {
            continue;
        }
        // 绝对路径：只有「可执行文件」才算程序（命令里的数据文件路径不是解释器——
        // 把它当解释器会连带把那个文件放行，等于给围栏开了个洞）。
        let p = std::path::Path::new(token);
        if p.is_absolute() {
            if p.is_file() && is_executable(p) {
                dirs.push(p.to_path_buf());
            }
            continue;
        }
        candidates.push(token.to_string());
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| String::new())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    for name in candidates {
        for dir in std::env::split_paths(&path_var) {
            let mut tries: Vec<std::path::PathBuf> = vec![dir.join(&name)];
            for e in &exts {
                tries.push(dir.join(format!("{}{}", name, e.to_lowercase())));
                tries.push(dir.join(format!("{}{}", name, e)));
            }
            if let Some(hit) = tries.into_iter().find(|p| p.is_file() && is_executable(p)) {
                // 解释器常见布局：<root>/bin/xxx（官方安装与虚拟环境）或 <root>/xxx。
                // 只授它自己的安装目录：<root>/bin 这种布局上溯一层（标准库在 <root> 里），其余用所在目录。
                if let Some(parent) = hit.parent() {
                    let leaf = parent
                        .file_name()
                        .map(|s| s.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    let target = if leaf == "bin" || leaf == "scripts" {
                        parent.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| parent.to_path_buf())
                    } else {
                        parent.to_path_buf()
                    };
                    dirs.push(target);
                }
                break;
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// 是不是「可执行文件」：Unix 看执行位；Windows 没有这个概念，文件存在即算（PATH 解析已按 PATHEXT 试过扩展名）。
#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &std::path::Path) -> bool {
    true
}

/// 祖先目录（不含自己）：受限进程要按名穿过它们才能到达被放行的根
/// （Windows 授 FILE_TRAVERSE，macOS 放行只读元数据）。
pub(crate) fn ancestors_of(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let mut cur = path.parent();
    while let Some(p) = cur {
        out.push(p.to_path_buf());
        cur = p.parent();
    }
    out
}

/// 工具进程的启动命令：命令行由**模块作者**写在 module.yaml 里，交系统 shell 解释（与既有语义一致）。
#[cfg(windows)]
pub fn shell_command(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(command);
    c
}

#[cfg(not(windows))]
pub fn shell_command(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// 环境白名单：子进程只拿到这些（其余一律不继承——密钥与无关凭据不进工具进程）。
/// 解释器需要 HOME/TEMP 这类落点：全部指到该 agent 的私有沙箱里（缓存与临时文件落在工作区内）。
pub fn fence_env(spec: &FenceSpec) -> Vec<(OsString, OsString)> {
    let keep = [
        "PATH", "PATHEXT", "SystemRoot", "WINDIR", "COMSPEC", "ComSpec", "SYSTEMDRIVE",
        "LANG", "LC_ALL", "TZ",
    ];
    let mut out: Vec<(OsString, OsString)> = Vec::new();
    for k in keep {
        if let Some(v) = std::env::var_os(k) {
            out.push((OsString::from(k), v));
        }
    }
    // 文本编码统一 UTF-8（Windows 上 Python 默认按系统代码页解 stdin，会把中文参数解坏）。
    out.push((OsString::from("PYTHONIOENCODING"), OsString::from("utf-8")));
    out.push((OsString::from("PYTHONUTF8"), OsString::from("1")));
    // 工作区内的落点：私有沙箱作为 HOME / TEMP（缓存与临时文件不出工作区）。
    let home = spec.private_or_cwd();
    out.push((OsString::from("HOME"), home.clone().into_os_string()));
    out.push((OsString::from("USERPROFILE"), home.clone().into_os_string()));
    out.push((OsString::from("TEMP"), home.clone().into_os_string()));
    out.push((OsString::from("TMP"), home.into_os_string()));
    out
}

impl FenceSpec {
    /// 该 agent 的私有沙箱（没有就退回工作目录）——环境里的 HOME / TEMP 落点。
    pub fn private_or_cwd(&self) -> PathBuf {
        self.rw.get(1).cloned().unwrap_or_else(|| self.cwd.clone())
    }
}
