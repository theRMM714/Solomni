//! 目的：命令里的程序解析、解释器安装根、shell 启动与环境白名单。
//! 管：PATH / PATHEXT 解析、安装目录上溯、围栏环境；平台分支在 `paths/windows.rs` / `paths/unix.rs`。
//! 不管：围栏授权与台账、平台容器生命周期。
//! 联动：由 `prep.rs`、`process.rs` 与 `entry/toolchains.rs` 消费。

use crate::kernel::api::FenceSpec;
use std::ffi::OsString;
use std::process::Command;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as platform;

#[cfg(windows)]
pub(crate) use windows::{
    NODE_REALPATH_FLAGS, container_program_command, view_program_command,
    windows_program_separators,
};
#[cfg(all(windows, test))]
pub(super) use windows::{canonical_path_var, node_realpath_env_in};

/// 目的：命令里可能出现的外部程序：按 PATH 解析出真实路径（解析不出的跳过，不猜）。
///   它们的**安装目录**必须放行（只读+执行），否则受限进程连解释器都起不来——Windows 的目录 ACL 与 macOS 的 seatbelt 都靠它。
pub(crate) fn interpreter_dirs(command: &str) -> Vec<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    interpreter_dirs_in(command, &path_var, pathext.as_deref())
}

/// 可执行扩展名：PATHEXT（若在场）**加**一份标准兜底，按小写去重。
/// 兜底不是装饰：PATHEXT 可能缺席（CI 的 runner 起 node 再起产品），
/// 那时只按 PATHEXT 找扩展名会一个解释器都解析不出来——容器里的工具连 python 都找不到。
fn exec_extensions(pathext: Option<&str>) -> Vec<String> {
    let mut exts: Vec<String> = Vec::new();
    let mut push = |ext: &str| {
        let ext = ext.trim().to_ascii_lowercase();
        if !ext.is_empty() && !exts.contains(&ext) {
            exts.push(ext);
        }
    };
    for ext in [".exe", ".cmd", ".bat", ".com"] {
        push(ext);
    }
    if let Some(text) = pathext {
        for ext in text.split(';') {
            push(ext);
        }
    }
    exts
}

/// 命令里出现的**程序**：按 PATH 解析出真实路径（解析不出的跳过，不猜）。
/// 约束：绝对路径的 token 一律不算——那可能是数据文件，把它当程序会连带把它放行（见 interpreter_dirs 的用例）。
/// 约束：解释器目录与「命令用到了哪个解释器」都从这一份派生——两处各写一遍迟早会漏掉某一类。
fn command_programs_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let mut candidates: Vec<String> = Vec::new();
    for raw in command.split([' ', '\t', '&', '|', ';', '\n']) {
        let token = raw.trim_matches(|c| c == '"' || c == '\'' || c == '(' || c == ')');
        if token.is_empty()
            || token.starts_with('-')
            || token.starts_with('/')
            || token.starts_with('%')
            || std::path::Path::new(token).is_absolute()
        {
            continue;
        }
        candidates.push(token.to_string());
    }
    let exts = exec_extensions(pathext);
    let mut hits: Vec<std::path::PathBuf> = Vec::new();
    for name in candidates {
        for entry in std::env::split_paths(path_var) {
            // PATH 项可能带外层引号（手写的 PATH 常见）：带引号去 join 就永远找不到。
            let text = entry.to_string_lossy();
            let unquoted = text.trim_matches('"');
            let dir = if unquoted.len() == text.len() {
                entry
            } else {
                std::path::PathBuf::from(unquoted.to_string())
            };
            let mut tries: Vec<std::path::PathBuf> = vec![dir.join(&name)];
            for e in &exts {
                tries.push(dir.join(format!("{}{}", name, e)));
            }
            if let Some(hit) = tries
                .into_iter()
                .find(|p| p.is_file() && platform::is_executable(p))
            {
                hits.push(hit);
                break;
            }
        }
    }
    hits
}

/// 目的：按调用方喂进来的环境形状解析解释器目录（**不能假设 PATH / PATHEXT 一定在场或一定干净**）。
pub(super) fn interpreter_dirs_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    for hit in command_programs_in(command, path_var, pathext) {
        dirs.extend(interpreter_roots_for_exe(&hit));
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// 目的：解释器程序 → 要授权的安装根（Windows 按真身；Unix 链接与真身都留）。
pub(crate) fn interpreter_roots_for_exe(exe: &std::path::Path) -> Vec<std::path::PathBuf> {
    platform::interpreter_roots_for_exe(exe)
}

/// 目的：这个解释器安装目录在**平台围栏**下能不能放行（只读探测，不写权限项）。
pub(crate) fn interpreter_dir_usable(dir: &std::path::Path) -> Result<(), String> {
    platform::interpreter_dir_usable(dir)
}

/// 目的：程序所在目录 → 该程序的「安装目录」：<root>/bin|Scripts 这种布局上溯一层，其余就是所在目录。
pub(super) fn install_dir(bin: &std::path::Path) -> std::path::PathBuf {
    let leaf = bin
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if leaf == "bin" || leaf == "scripts" {
        // 绝不把「文件系统根」当安装目录：上溯到根等于把整盘放行。
        match bin.parent() {
            Some(up) if up.parent().is_some() => up.to_path_buf(),
            _ => bin.to_path_buf(),
        }
    } else {
        bin.to_path_buf()
    }
}

/// 目的：工具进程的启动命令：命令行由**模块作者**写在 module.yaml 里，交系统 shell 解释。
pub fn shell_command(command: &str) -> Command {
    platform::shell_command(command)
}

/// 目的：环境白名单：子进程只拿到这些（其余一律不继承——密钥与无关凭据不进工具进程）。
///   解释器需要 HOME/TEMP 这类落点：全部指到该 agent 的私有沙箱里。
pub fn fence_env(spec: &FenceSpec, command: &str) -> Vec<(OsString, OsString)> {
    let keep = [
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "ComSpec",
        "SYSTEMDRIVE",
        "LANG",
        "LC_ALL",
        "TZ",
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
    // Windows 建 AppContainer 进程要读它：白名单里没有它就 CreateProcessW 直接失败。
    out.push((
        OsString::from("LOCALAPPDATA"),
        home.clone().into_os_string(),
    ));
    out.push((OsString::from("USERPROFILE"), home.clone().into_os_string()));
    out.push((OsString::from("TEMP"), home.clone().into_os_string()));
    out.push((OsString::from("TMP"), home.clone().into_os_string()));
    // macOS / Linux 认 TMPDIR：不设的话进程会去读系统临时区（那不在可达范围内）。
    out.push((OsString::from("TMPDIR"), home.into_os_string()));
    platform::adjust_env(&mut out, command);
    out
}
