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

/// 本机能不能强制住这次执行的围栏——**机制层的验证结论**，与"外层授权了没有"无关。
/// 三态的理由：把"本机不允许"与"我们的机制写错了"分开。
/// 前者是环境结论，如实降级照跑（能力等级已在启动报告里说过）；后者绝不能静默降级——
/// 那等于用户以为有围栏、实际什么都没有。探针早就按这两类分别处理，运行期也必须一样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceVerdict {
    /// 机制真装上了，强制生效。
    Enforced,
    /// 本机环境不允许装（内核不支持、私有 ABI 失效、系统拒绝建容器）——降级照跑，不是我们的错。
    EnvUnavailable(String),
    /// 自检已确认机制有效，但我们的规则/步骤装不上 = 我们写错了。未授权时据此**拒绝执行**。
    Broken(String),
}

/// 本机能不能强制住这次执行的围栏（机制层自检，**不写本机任何权限项**）。
/// 未授权时段靠它把"环境不允许"与"我们写错了"分开——后者绝不能被当成降级吞掉。
pub fn verify(spec: &FenceSpec, command: &str) -> FenceVerdict {
    backend::verify(spec, command)
}

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

/// 守门进程的入参：core 的围栏策略 + **外层是否已把本机授权做完** + 产品私有区（台账落点）。
/// 授权是改本机目录 ACL 的动作（只有 Windows 的容器围栏需要），所以它不进 core 的 `FenceSpec`，
/// 由适配层随这次执行一起交给守门进程。JSON 是**扁平**的：FenceSpec 的字段同层再加 `prepared` 与 `home`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceJob {
    pub spec: FenceSpec,
    pub prepared: bool,
    /// 产品私有区（`.home/`）：守门进程把**它建过的容器 profile** 记进这里的台账，供 `--fence-clean` 精确回收。
    /// 守门进程是唯一真正建 profile 的地方，外层只知道"该建"、不知道"建成了"。
    /// 没有台账的调用方（探针）给 `None`：那类 profile 由 `--fence-clean` 的前缀清扫兜底。
    pub home: Option<PathBuf>,
}

impl FenceJob {
    pub fn to_json(&self) -> String {
        let mut fields = match serde_json::to_value(&self.spec) {
            Ok(serde_json::Value::Object(o)) => o,
            _ => serde_json::Map::new(),
        };
        fields.insert("prepared".to_string(), serde_json::Value::Bool(self.prepared));
        fields.insert("home".to_string(), serde_json::to_value(&self.home).unwrap_or(serde_json::Value::Null));
        serde_json::Value::Object(fields).to_string()
    }

    pub fn from_json(text: &str) -> Result<FenceJob, String> {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(flatten)]
            spec: FenceSpec,
            /// 缺了这一项 = 调用方没说清有没有授权，如实报错（不默认成"有"）。
            prepared: bool,
            /// 缺省 = 没有台账可落（探针、别的调用方）；这不是"忘记传"，所以允许缺省。
            #[serde(default)]
            home: Option<PathBuf>,
        }
        serde_json::from_str::<Raw>(text)
            .map(|raw| FenceJob { spec: raw.spec, prepared: raw.prepared, home: raw.home })
            .map_err(|e| format!("围栏参数非法：{}", e))
    }
}

/// 组装守门进程的命令行：工具命令作为**数据**传递（不拼进 shell 字符串，杜绝注入）。
pub fn launcher(exe: &Path, job: &FenceJob, command: &str) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg(FENCE_FLAG).arg(job.to_json()).arg("--").arg(command);
    cmd
}

/// 守门进程内：装围栏 → 跑命令 → 返回退出码。失败必须报错（stderr）并用 FENCE_FAILED 退出。
pub fn run_fenced(job: &FenceJob, command: &str) -> i32 {
    backend::run_fenced(&job.spec, job.prepared, job.home.as_deref(), command)
}

/// 扫掉本程序建过的整族容器 profile：台账只记"我们知道写过什么"，而 profile 可能来自没有台账的路径
/// （探针、夹具的台账被删、旧版本）。名字前缀是本程序独有的，所以按它扫。返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    #[cfg(windows)]
    {
        windows::sweep_profiles()
    }
    #[cfg(not(windows))]
    {
        // 其它平台没有容器 profile 这一步。
        Ok(0)
    }
}

/// 外层进程调用：把围栏要用的授权一次性做好（写目录 ACL）；prepared 是"已经授权过"的台账，
/// 避免每次工具调用重复改 ACL。
/// 只有 Windows 的容器围栏需要这一步——Linux 的 Landlock 与 macOS 的 seatbelt 在守门进程里自足，
/// 所以本函数在非 Windows 平台上**不存在**（而不是"存在但空转"）。
#[cfg(windows)]
pub fn prepare_fence(
    spec: &FenceSpec,
    command: &str,
    prepared: &std::sync::Mutex<std::collections::BTreeSet<String>>,
    home: &std::path::Path,
) -> Result<(), String> {
    windows::prepare_fence(spec, command, prepared, home)
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
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    interpreter_dirs_in(command, &path_var, pathext.as_deref())
}

/// 可执行扩展名：PATHEXT（若在场）**加**一份标准兜底，按小写去重。
/// 兜底不是装饰：真机上见过 PATHEXT 缺席的进程环境（CI 的 runner 起 node 再起产品），
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

/// 去掉 Windows 规范化路径的 `\\?\` 前缀：安全描述符接口不认这个前缀，带上就是白写一条授权。
fn strip_verbatim_prefix(path: &std::path::Path) -> std::path::PathBuf {
    match path.to_string_lossy().strip_prefix(r"\\?\") {
        Some(rest) => std::path::PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// 解析规则由调用方把环境形状喂进来：**不能假设 PATH / PATHEXT 一定在场或一定干净**。
fn interpreter_dirs_in(
    command: &str,
    path_var: &std::ffi::OsStr,
    pathext: Option<&str>,
) -> Vec<std::path::PathBuf> {
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
    let exts = exec_extensions(pathext);
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
                // 解释器常见布局：<root>/bin/xxx（官方安装与虚拟环境）或 <root>/xxx。
                if let Some(parent) = hit.parent() {
                    dirs.push(install_dir(parent));
                }
                // 符号链接要把**真身**的安装目录也放行：macOS 上 python3 常常是链接，动态库在真身旁边——
                // 只放行链接所在目录会让加载器取不到库（进程直接 SIGABRT）。
                if let Ok(real) = std::fs::canonicalize(&hit) {
                    let real = strip_verbatim_prefix(&real);
                    if real != hit {
                        if let Some(parent) = real.parent() {
                            dirs.push(install_dir(parent));
                        }
                    }
                }
                break;
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// 程序所在目录 → 该程序的「安装目录」：<root>/bin|Scripts 这种布局上溯一层（标准库与动态库在 <root> 里），其余就是所在目录。
fn install_dir(bin: &std::path::Path) -> std::path::PathBuf {
    let leaf = bin
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if leaf == "bin" || leaf == "scripts" {
        if let Some(up) = bin.parent() {
            // 绝不把「文件系统根」当安装目录：/bin 的父目录就是 /，
            // 上溯到根等于把整盘放行（macOS 上会直接让围栏形同虚设）。
            if up.parent().is_some() {
                return up.to_path_buf();
            }
        }
    }
    bin.to_path_buf()
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

/// 交给 cmd 解释前，把**程序名**里的正斜杠换成反斜杠。
/// cmd 只把程序名里的 `\` 当路径分隔符：`build/indexer build` 会被它读成「命令 build + 开关 /indexer」，
/// 报 `'build' is not recognized`。程序名之后的参数原样保留（`node tools/report.js` 这类命令靠参数里的正斜杠）。
/// 程序名 = 第一个空白前的字段；模块作者若用引号包住程序名，只改引号内那一段。
#[cfg(windows)]
pub(crate) fn windows_program_separators(command: &str) -> String {
    let end = match command.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|i| i + 2).unwrap_or(command.len()),
        None => command.find(char::is_whitespace).unwrap_or(command.len()),
    };
    command[..end].replace('/', "\\") + &command[end..]
}

/// 工具进程的启动命令：命令行由**模块作者**写在 module.yaml 里，交系统 shell 解释（与既有语义一致）。
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
    // Windows 建 AppContainer 进程要读它：白名单里没有它就 CreateProcessW 直接失败（os error 203），
    // 容器整条路会静默降级成无围栏执行。落点同样指进该 agent 的私有沙箱。
    out.push((OsString::from("LOCALAPPDATA"), home.clone().into_os_string()));
    out.push((OsString::from("USERPROFILE"), home.clone().into_os_string()));
    out.push((OsString::from("TEMP"), home.clone().into_os_string()));
    out.push((OsString::from("TMP"), home.clone().into_os_string()));
    // macOS / Linux 认 TMPDIR：不设的话进程会去读系统临时区（那不在可达范围内）。
    out.push((OsString::from("TMPDIR"), home.into_os_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 安装目录上溯**绝不能停在文件系统根**：/bin 的父目录就是 /，
    /// 一旦返回 / 就等于把整盘放行（macOS 的 seatbelt 会因此形同虚设，真机上已抓到过一次）。
    #[test]
    fn install_dir_never_climbs_to_the_filesystem_root() {
        let root = if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") };
        assert_eq!(install_dir(&root.join("bin")), root.join("bin"), "根下的 bin 不再上溯");
        assert_eq!(install_dir(&root), root, "根就是根");
        let deep = root.join("home").join("u").join(".venv").join("bin");
        assert_eq!(install_dir(&deep), root.join("home").join("u").join(".venv"), "普通布局上溯一层");
        let scripts = root.join("home").join("u").join("env").join("Scripts");
        assert_eq!(install_dir(&scripts), root.join("home").join("u").join("env"), "Scripts 布局同样上溯");
        assert_eq!(install_dir(&root.join("usr").join("local").join("bin")), root.join("usr").join("local"), "usr/local/bin 上溯到 usr/local");
    }

    /// cmd 只认程序名里的反斜杠：`build/indexer build` 会被读成命令 build + 开关 /indexer（真机上模块工具因此跑不起来）。
    /// 参数里的正斜杠必须原样保留——`node tools/report.js` 正是靠它。
    #[cfg(windows)]
    #[test]
    fn windows_program_separators_rewrites_only_the_program_name() {
        assert_eq!(windows_program_separators("build/indexer build"), "build\\indexer build");
        assert_eq!(windows_program_separators("node tools/report.js"), "node tools/report.js");
        assert_eq!(windows_program_separators("python tools/scan.py extra"), "python tools/scan.py extra");
        assert_eq!(windows_program_separators(".tools/mingw64/bin/g++.exe -O2"), ".tools\\mingw64\\bin\\g++.exe -O2");
        assert_eq!(windows_program_separators("build\\indexer build"), "build\\indexer build");
        assert_eq!(windows_program_separators("\"a/b\" rest"), "\"a\\b\" rest");
        assert_eq!(windows_program_separators("plain"), "plain");
    }

    /// 守门进程的入参是**扁平** JSON：FenceSpec 的字段同层再加一个 prepared；
    /// 缺字段一律报错（不默认成"有授权"——那会把容器送进一个读不到东西的环境）。
    #[test]
    fn fence_job_round_trips_and_rejects_incomplete_json() {
        let spec = FenceSpec {
            agent: "a".to_string(),
            rw: vec![PathBuf::from("demo").join("work")],
            cwd: PathBuf::from("mods").join("m0"),
            net: false,
        };
        let job = FenceJob { spec, prepared: true, home: Some(PathBuf::from("home")) };
        let text = job.to_json();
        assert!(text.contains("\"prepared\":true"), "{}", text);
        assert_eq!(FenceJob::from_json(&text).expect("回读守门进程入参"), job);
        // 没有台账可落是合法形态（探针），所以 home 允许缺省；prepared 缺了才报错。
        let bare = FenceSpec {
            agent: "b".to_string(),
            rw: vec![PathBuf::from("demo").join("work")],
            cwd: PathBuf::from("mods").join("m1"),
            net: true,
        };
        let no_home = FenceJob { spec: bare, prepared: false, home: None };
        assert_eq!(FenceJob::from_json(&no_home.to_json()).expect("回读"), no_home);
        assert!(FenceJob::from_json("{}").is_err(), "缺字段必须报错，不猜");
        assert!(FenceJob::from_json("这不是 JSON").is_err());
    }

    /// 解析解释器不能假设环境形状：PATHEXT 缺席（真机 CI 上见过）时靠标准兜底扩展名照样找到 .exe，
    /// 带引号的 PATH 项照样能用——否则容器里的工具连解释器都找不到（真机上就是这么挂的）。
    #[test]
    fn interpreter_dirs_resolves_without_pathext_and_with_quoted_path_entries() {
        let root = crate::tests::scratch("interpreter-dirs");
        let pydir = root.join("pydir");
        std::fs::create_dir_all(&pydir).expect("建解释器目录");
        let exe = pydir.join(if cfg!(windows) { "python.exe" } else { "python" });
        std::fs::write(&exe, b"#!/bin/sh\nexit 0\n").expect("放一个假解释器");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(&exe).expect("读权限位").permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&exe, perm).expect("加执行位");
        }
        let path = pydir.as_os_str();
        assert_eq!(
            interpreter_dirs_in("python tools/x.py", path, None),
            vec![pydir.clone()],
            "PATHEXT 缺席也要解析出解释器目录"
        );
        assert_eq!(
            interpreter_dirs_in("python tools/x.py", path, Some(".EXE;.BAT")),
            vec![pydir.clone()],
            "PATHEXT 在场照走 PATHEXT"
        );
        let quoted = std::ffi::OsString::from(format!("\"{}\"", pydir.display()));
        assert_eq!(
            interpreter_dirs_in("python tools/x.py", &quoted, None),
            vec![pydir.clone()],
            "带引号的 PATH 项也要能解析"
        );
        assert!(
            interpreter_dirs_in(&format!("cat {}", exe.display()), path, None).iter().all(|d| d != &pydir),
            "数据文件路径不算解释器（否则等于给围栏开洞）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 命令里的解释器要按 PATH 解析出真实路径，并给出它的安装目录；
    /// 数据文件路径不算解释器（否则会把那个文件放行，等于开洞）。
    #[test]
    fn interpreter_dirs_resolves_programs_but_not_data_files() {
        let cmd = if cfg!(windows) { "cmd /C echo hi" } else { "sh -c 'echo hi'" };
        let dirs = interpreter_dirs(cmd);
        assert!(!dirs.is_empty(), "系统 shell 应当能被解析出来：{:?}", dirs);
        for d in &dirs {
            assert!(d.is_dir(), "只报真实存在的目录：{:?}", d);
            assert!(d.parent().is_some(), "绝不报文件系统根：{:?}", d);
        }
        // 明确的非程序路径（一个不存在的文件）不该被当成解释器。
        let missing = if cfg!(windows) { "C:\\nope\\nope.exe" } else { "/nope/nope" };
        assert!(
            interpreter_dirs(&format!("cat {}", missing)).iter().all(|d| !d.ends_with("nope")),
            "数据/缺失路径不该被当成解释器"
        );
    }
}

impl FenceSpec {
    /// 该 agent 的私有沙箱（没有就退回工作目录）——环境里的 HOME / TEMP 落点。
    pub fn private_or_cwd(&self) -> PathBuf {
        self.rw.get(1).cloned().unwrap_or_else(|| self.cwd.clone())
    }
}
