//! 目的：`paths` 的 Windows 后端——真身解析、重解析点判定、node 开关与 cmd 命令行。
//! 管：解释器安装根、目录可用性探测、PATH 真身化、程序名改写与 shell 选择。
//! 不管：围栏授权与台账；环境白名单的公共部分（在门面）。
//! 联动：由 `src/kernel/detail/confine/paths.rs` 按平台选中。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::kernel::detail::confine::windows as confine_backend;
use crate::kernel::detail::view_drive::ViewDrive;

/// 目的：Windows 没有执行位，文件存在即算（PATH 解析已按 PATHEXT 试过扩展名）。
pub(super) fn is_executable(_path: &Path) -> bool {
    true
}

/// 目的：解释器程序 → 要授权的安装根（Windows 按真身；链接本身不授）。
/// 约束：父目录是重解析点时只授真身——容器里的 PATH 项已经被换写成真身，链接对象因此不需要 DACL。
pub(super) fn interpreter_roots_for_exe(exe: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let parent = exe.parent();
    let real = std::fs::canonicalize(exe)
        .ok()
        .map(|p| crate::kernel::domain::path::native(&p));
    if let Some(p) = parent.filter(|p| !is_reparse_point(p)) {
        dirs.push(super::install_dir(p));
    }
    if let Some(p) = real.as_deref().and_then(|real| real.parent()) {
        dirs.push(super::install_dir(p));
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// 目的：路径本身是不是重解析点（符号链接 / 联接）。
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

/// 目的：把 PATH 里的绝对目录项换成真身（逐层解析 junction / 符号链接），容器因此不再按链接原路径搜索。
pub(crate) fn canonical_path_var(path_var: &OsStr) -> OsString {
    let mut out: Vec<OsString> = Vec::new();
    for entry in std::env::split_paths(path_var) {
        let text = entry.to_string_lossy();
        let unquoted = text.trim_matches('"');
        let candidate = if unquoted.len() == text.len() {
            entry.clone()
        } else {
            PathBuf::from(unquoted)
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

/// 目的：这个解释器安装目录在平台围栏下能不能放行（只读探测，不写权限项）。
pub(super) fn interpreter_dir_usable(dir: &Path) -> Result<(), String> {
    confine_backend::interpreter_dir_usable(dir)
}

/// 目的：node 的「跳过 realpath」开关——主模块看 `-main`，依赖与 ESM 看另一个（两个都要）。
pub(crate) const NODE_REALPATH_FLAGS: &str = "--preserve-symlinks --preserve-symlinks-main";

/// 目的：命令用 node 时要补的环境（`NODE_OPTIONS` 跳过 realpath）；不是 node 就是 `None`。
fn node_realpath_env(command: &str) -> Option<(OsString, OsString)> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    node_realpath_env_in(command, &path_var, pathext.as_deref())
}

/// 目的：按调用方喂进来的环境形状判断命令是否用 node，并给出 `NODE_OPTIONS` 基线。
pub(crate) fn node_realpath_env_in(
    command: &str,
    path_var: &OsStr,
    pathext: Option<&str>,
) -> Option<(OsString, OsString)> {
    let is_node = super::command_programs_in(command, path_var, pathext)
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

/// 目的：程序是不是 node：看程序名，也看真身的程序名（安装器常做成符号链接）。
fn is_node_program(program: &Path) -> bool {
    fn named_node(p: &Path) -> bool {
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

/// 目的：把命令的**程序名**改写成相对 cwd 的路径——AppContainer 里绝对路径的程序会被拒。
pub(crate) fn container_program_command(command: &str, cwd: &Path) -> String {
    let (head, rest) = split_program(command);
    let bare = head.trim_matches('"');
    if bare.is_empty() {
        return command.to_string();
    }
    // 只处理明确是程序路径的首 token：绝对路径或带分隔符的相对路径。裸名交给 cmd 自己解析。
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
fn split_program(command: &str) -> (&str, &str) {
    let end = match command.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|i| i + 2).unwrap_or(command.len()),
        None => command.find(char::is_whitespace).unwrap_or(command.len()),
    };
    (&command[..end], &command[end..])
}

/// 目的：从 `base` 到 `target` 的相对路径；不同卷或算不出给 `None`。
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
pub(crate) fn windows_program_separators(command: &str) -> String {
    let end = match command.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|i| i + 2).unwrap_or(command.len()),
        None => command.find(char::is_whitespace).unwrap_or(command.len()),
    };
    command[..end].replace('/', "\\") + &command[end..]
}

/// 目的：Windows 的工具进程用 `cmd /C` 解释命令行。
pub(super) fn shell_command(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(windows_program_separators(command));
    c
}

/// 目的：把 `PATH` 换成真身，并在命令用 node 时补上 `NODE_OPTIONS`。
pub(super) fn adjust_env(out: &mut Vec<(OsString, OsString)>, command: &str) {
    for (k, v) in out.iter_mut() {
        if k.to_str() == Some("PATH") {
            *v = canonical_path_var(v);
        }
    }
    if let Some((k, v)) = node_realpath_env(command) {
        out.push((k, v));
    }
}
