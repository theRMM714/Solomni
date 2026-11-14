//! 目的：命令里的程序解析、解释器安装根、shell 启动与环境白名单。
//! 管：PATH / PATHEXT 解析、安装目录上溯、node realpath 开关、Windows 程序名改写、围栏环境。
//! 不管：围栏授权与台账、平台容器生命周期。
//! 联动：由 `prep.rs`、`process.rs` 与 `entry/toolchains.rs` 消费；平台差异集中在 `windows/`。

use crate::kernel::api::FenceSpec;
use std::ffi::OsString;
#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use super::windows;
#[cfg(windows)]
use crate::kernel::detail::view_drive::ViewDrive;

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
/// 约束：解释器目录与「命令用到了哪个解释器」都从这一份派生——两处各写一遍迟早会漏掉某一类
///   （PATHEXT 缺席、带引号的 PATH 项、符号链接的真身）。
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
            if let Some(hit) = tries.into_iter().find(|p| p.is_file() && is_executable(p)) {
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
/// 约束：父目录是重解析点时只授真身——容器里的 PATH 项已经被换写成真身（`canonical_path_var`），
///   不再按 junction 原路径搜索，链接对象因此不需要 DACL（给链接写 DACL 会被写后核对判成丢项）。
///   父目录是实目录时授它：程序本身可能是文件级链接，链接旁的兄弟文件也要能读。
///   真身取不到且父目录是重解析点：不授链接（容器里也不会按它搜索），让授权按真实失败如实报。
pub(crate) fn interpreter_roots_for_exe(exe: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let parent = exe.parent();
    let real = std::fs::canonicalize(exe)
        .ok()
        .map(|p| crate::kernel::domain::path::native(&p));
    #[cfg(windows)]
    {
        if let Some(p) = parent.filter(|p| !is_reparse_point(p)) {
            dirs.push(install_dir(p));
        }
        if let Some(p) = real.as_deref().and_then(|real| real.parent()) {
            dirs.push(install_dir(p));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(p) = parent {
            dirs.push(install_dir(p));
        }
        if let Some(p) = real
            .as_deref()
            .filter(|real| *real != exe)
            .and_then(|real| real.parent())
        {
            dirs.push(install_dir(p));
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// 目的：路径本身是不是重解析点（符号链接 / 联接）。
#[cfg(windows)]
fn is_reparse_point(path: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

/// 目的：把 PATH 里的绝对目录项换成真身（逐层解析 junction / 符号链接），容器因此不再按链接原路径搜索。
/// 约束：只改写能 canonicalize 的绝对目录项；相对项、取不到真身的项原样保留，顺序不变。
#[cfg(windows)]
pub(super) fn canonical_path_var(path_var: &std::ffi::OsStr) -> std::ffi::OsString {
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for entry in std::env::split_paths(path_var) {
        let text = entry.to_string_lossy();
        let unquoted = text.trim_matches('"');
        let candidate = if unquoted.len() == text.len() {
            entry.clone()
        } else {
            std::path::PathBuf::from(unquoted)
        };
        let resolved = if candidate.is_absolute() {
            std::fs::canonicalize(&candidate)
                .ok()
                .map(|p| crate::kernel::domain::path::native(&p))
                .filter(|p| p.is_dir())
        } else {
            None
        };
        out.push(
            resolved
                .map(|p| p.into_os_string())
                .unwrap_or_else(|| entry.into_os_string()),
        );
    }
    std::env::join_paths(out).unwrap_or_else(|_| path_var.to_os_string())
}

/// 目的：这个解释器安装目录在**平台围栏**下能不能放行（只读探测，不写权限项）。
///   Windows：已有足够的只读基线 ACE（本产品能力 SID，或系统目录自带的全应用包），或目录属主是当前用户（能写 DACL）；
///   其它平台：围栏只读放行，目录存在即可。
pub(crate) fn interpreter_dir_usable(dir: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::interpreter_dir_usable(dir)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Ok(())
    }
}

/// 目的：程序所在目录 → 该程序的「安装目录」：<root>/bin|Scripts 这种布局上溯一层（标准库与动态库在 <root> 里），其余就是所在目录。
pub(super) fn install_dir(bin: &std::path::Path) -> std::path::PathBuf {
    let leaf = bin
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if leaf == "bin" || leaf == "scripts" {
        // 绝不把「文件系统根」当安装目录：/bin 的父目录就是 /，
        // 上溯到根等于把整盘放行（macOS 上会直接让围栏形同虚设）。
        match bin.parent() {
            Some(up) if up.parent().is_some() => up.to_path_buf(),
            _ => bin.to_path_buf(),
        }
    } else {
        bin.to_path_buf()
    }
}

/// 目的：node 的「跳过 realpath」开关——主模块看 `-main`，依赖与 ESM 看另一个（两个都要）。
/// 约束：单开任何一个，另一半照样 realpath（真机实测如此）。
#[cfg(windows)]
pub(crate) const NODE_REALPATH_FLAGS: &str = "--preserve-symlinks --preserve-symlinks-main";

/// 目的：命令用 node 时要补的环境（`NODE_OPTIONS` 跳过 realpath）；不是 node 就是 `None`。
/// 约束：Windows 的容器围栏才需要它——`fs.realpathSync` 先 lstat 盘卷根、再逐级 lstat 祖先前缀，
///   而这两类落点按设计都不在可达范围（卷根属主是系统、非管理员改不动；祖先链只靠令牌的
///   「按名穿过」特权，管不到显式 lstat），进程在脚本执行前就 EPERM 死。
/// 约束：另两个平台不需要：Landlock 的权限位里没有「读属性」这一项，seatbelt 的规则显式给祖先
///   放行了 file-read-metadata（见 macos.rs 的祖先元数据放行）。
#[cfg(windows)]
fn node_realpath_env(command: &str) -> Option<(OsString, OsString)> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    node_realpath_env_in(command, &path_var, pathext.as_deref())
}

/// 目的：按调用方喂进来的环境形状判断命令是否用 node，并给出 `NODE_OPTIONS` 基线（与 `interpreter_dirs_in` 同一个理由：不能假设 PATH / PATHEXT 干净）。
#[cfg(windows)]
pub(super) fn node_realpath_env_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Option<(OsString, OsString)> {
    let is_node = command_programs_in(command, path_var, pathext)
        .iter()
        .any(|p| is_node_program(p));
    if !is_node {
        return None;
    }
    Some((
        OsString::from("NODE_OPTIONS"),
        OsString::from(NODE_REALPATH_FLAGS),
    ))
}

/// 程序是不是 node：看程序名，也看**真身**的程序名——安装器常把 `<root>/nodejs` 做成指向具体版本目录的
/// 符号链接（真机如此），只认链接名会漏掉那一类。
#[cfg(windows)]
fn is_node_program(program: &std::path::Path) -> bool {
    fn named_node(p: &std::path::Path) -> bool {
        p.file_stem()
            .map(|s| s.to_string_lossy().eq_ignore_ascii_case("node"))
            .unwrap_or(false)
    }
    if named_node(program) {
        return true;
    }
    std::fs::canonicalize(program)
        .map(|real| named_node(&crate::kernel::domain::path::native(&real)))
        .unwrap_or(false)
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

/// 目的：把命令的**程序名**改写成相对 cwd 的路径——AppContainer 里绝对路径的程序会被拒（真机：同一文件
///   `D:\…\tool.exe` 报 Access denied，`..\tool.exe` 正常；相对路径基于 cwd 的目录句柄解析，不遍历卷根）。
/// 参数：`command` = 模块声明的启动命令；`cwd` = 工具进程的工作目录；`path_var` / `pathext` = 容器里生效的解析规则。
/// 返回：改写后的命令；程序名解析不出、跨卷、或算不出相对路径时**原样返回**（行为不变）。
/// 约束：只改**首 token**（与 `windows_program_separators` 同一口径），参数里的路径不动。
#[cfg(windows)]
pub(crate) fn container_program_command(command: &str, cwd: &Path) -> String {
    let (head, rest) = split_program(command);
    let bare = head.trim_matches('"');
    if bare.is_empty() {
        return command.to_string();
    }
    // 只处理**明确是程序路径**的首 token：绝对路径或带分隔符的相对路径。裸名不碰——cmd 的内建命令与
    // PATH 里的同名程序分不开（真机 CI 上 `echo` 曾被改写成 Git 的 `echo.exe`），交给 cmd 自己解析。
    let abs = if Path::new(bare).is_absolute() {
        PathBuf::from(bare)
    } else if bare.contains(['\\', '/']) {
        cwd.join(bare)
    } else {
        return command.to_string();
    };
    let Some(rel) = relative_to(cwd, &abs) else {
        return command.to_string();
    };
    let text = rel.to_string_lossy().into_owned();
    let quoted = if text.contains(' ') {
        format!("\"{}\"", text)
    } else {
        text
    };
    format!("{}{}", quoted, rest)
}

/// 目的：命令的程序名若在产品根内，改写成视图盘写法——容器用视图 cwd，程序路径也必须落在视图域内。
/// 参数：command = 模块声明的启动命令；view = 本次生效的视图盘。
/// 约束：只改首 token；根外路径、相对路径与裸名原样返回（语义与 container_program_command 一致）。
#[cfg(windows)]
pub(crate) fn view_program_command(command: &str, view: &ViewDrive) -> String {
    let (head, rest) = split_program(command);
    let bare = head.trim_matches('"');
    if bare.is_empty() || !Path::new(bare).is_absolute() {
        return command.to_string();
    }
    let Some(viewed) = view.view_of(Path::new(bare)) else {
        return command.to_string();
    };
    let text = viewed.to_string_lossy().into_owned();
    let quoted = if head.starts_with('"') || text.contains(' ') {
        format!("\"{}\"", text)
    } else {
        text
    };
    format!("{}{}", quoted, rest)
}

/// 目的：命令拆成（程序名, 余下原文）；引号包住的程序名整段算一个 token。
#[cfg(windows)]
fn split_program(command: &str) -> (&str, &str) {
    let end = match command.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|i| i + 2).unwrap_or(command.len()),
        None => command.find(char::is_whitespace).unwrap_or(command.len()),
    };
    (&command[..end], &command[end..])
}

/// 目的：从 `base` 到 `target` 的相对路径；不同卷或算不出给 `None`。
#[cfg(windows)]
fn relative_to(base: &Path, target: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let b: Vec<Component> = base.components().collect();
    let t: Vec<Component> = target.components().collect();
    if b.is_empty() || t.is_empty() {
        return None;
    }
    let same = |x: &Component, y: &Component| {
        x.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&y.as_os_str().to_string_lossy())
    };
    if !same(&b[0], &t[0]) {
        return None;
    }
    let mut i = 0;
    while i < b.len() && i < t.len() && same(&b[i], &t[i]) {
        i += 1;
    }
    let mut out = PathBuf::new();
    for _ in i..b.len() {
        out.push("..");
    }
    for c in &t[i..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 目的：交给 cmd 解释前，把**程序名**里的正斜杠换成反斜杠。
///   cmd 只把程序名里的 `\` 当路径分隔符：`build/indexer build` 会被它读成「命令 build + 开关 /indexer」，
///   报 `'build' is not recognized`。程序名之后的参数原样保留（`node tools/report.js` 这类命令靠参数里的正斜杠）。
///   程序名 = 第一个空白前的字段；模块作者若用引号包住程序名，只改引号内那一段。
#[cfg(windows)]
pub(crate) fn windows_program_separators(command: &str) -> String {
    let end = match command.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|i| i + 2).unwrap_or(command.len()),
        None => command.find(char::is_whitespace).unwrap_or(command.len()),
    };
    command[..end].replace('/', "\\") + &command[end..]
}

/// 目的：工具进程的启动命令：命令行由**模块作者**写在 module.yaml 里，交系统 shell 解释（与既有语义一致）。
#[cfg(windows)]
pub fn shell_command(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(windows_program_separators(command));
    c
}

#[cfg(not(windows))]
pub fn shell_command(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// 目的：环境白名单：子进程只拿到这些（其余一律不继承——密钥与无关凭据不进工具进程）。
///   解释器需要 HOME/TEMP 这类落点：全部指到该 agent 的私有沙箱里（缓存与临时文件落在工作区内）。
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
            // 容器按真身搜索解释器：PATH 里的 junction / 符号链接在进容器前解析掉，
            // 于是只需要授真身目录，不必给链接对象写 DACL（链接写后核对会被判成丢项）。
            #[cfg(windows)]
            let v = if k == "PATH" {
                canonical_path_var(&v)
            } else {
                v
            };
            out.push((OsString::from(k), v));
        }
    }
    // 文本编码统一 UTF-8（Windows 上 Python 默认按系统代码页解 stdin，会把中文参数解坏）。
    out.push((OsString::from("PYTHONIOENCODING"), OsString::from("utf-8")));
    out.push((OsString::from("PYTHONUTF8"), OsString::from("1")));
    // 工作区内的落点：私有沙箱作为 HOME / TEMP（缓存与临时文件不出工作区）。
    let home = spec.private_or_cwd();
    out.push((OsString::from("HOME"), home.clone().into_os_string()));
    // Windows 建 AppContainer 进程要读它：白名单里没有它就 CreateProcessW 直接失败（os error 203），
    // 容器整条路会静默降级成无围栏执行。落点同样指进该 agent 的私有沙箱。
    out.push((
        OsString::from("LOCALAPPDATA"),
        home.clone().into_os_string(),
    ));
    out.push((OsString::from("USERPROFILE"), home.clone().into_os_string()));
    out.push((OsString::from("TEMP"), home.clone().into_os_string()));
    out.push((OsString::from("TMP"), home.clone().into_os_string()));
    // macOS / Linux 认 TMPDIR：不设的话进程会去读系统临时区（那不在可达范围内）。
    out.push((OsString::from("TMPDIR"), home.into_os_string()));
    // 解释器基线：命令用 node 时要跳过 realpath，否则容器里起不来（理由见 node_realpath_env）。
    #[cfg(windows)]
    if let Some((k, v)) = node_realpath_env(command) {
        out.push((k, v));
    }
    // 其余平台不需要它：Landlock 不管 stat、seatbelt 已放行祖先元数据，realpath 正常。
    #[cfg(not(windows))]
    let _ = command;
    out
}
