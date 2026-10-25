//! 目的：启动期把项目内基础工具链（.tools/toolchains）前置到 PATH，并设置工具需要的环境变量。
//! 管：读 active.json、校验目标平台与相对路径、拼出"项目 bin 在前、宿主 PATH 在后"的环境、产生回退报告。
//! 不管：下载 / 解包 / 版本选择（setup-toolchains.js）；guest 内工具链（后续阶段）；模块工具协议与围栏。
//! 联动：由 main 在根解析后调用；环境随后由 fence_env / interpreter_dirs / HostProbe 共用。

use crate::capabilities::packages::api::{BaseFacts, HostFallback};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// 目的：基础 profile 的工具 id（唯一真相是 toolchains.js 的 PROFILE；这里用用例钉住一致）。
pub(crate) const PROFILE: [&str; 3] = ["python", "node", "cc"];

/// 目的：当前宿主的目标名（与 setup-toolchains.js 的 TARGETS 同一套写法）。
pub(crate) fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        _ => None,
    }
}

#[derive(Debug, serde::Deserialize)]
struct ActiveFile {
    target: String,
    #[serde(default)]
    profile: Vec<String>,
    #[serde(default)]
    tools: Vec<ActiveTool>,
}

#[derive(Debug, serde::Deserialize)]
struct ActiveTool {
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    bin: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// 目的：一次启动期工具链应用的结果——启动报告用。
#[derive(Debug, Clone, Default)]
pub(crate) struct Applied {
    /// 目的：已生效的项目工具：(id, 版本)。
    pub active: Vec<(String, String)>,
    /// 目的：路径 / 解析问题（如实报告，不阻断启动）。
    pub warnings: Vec<String>,
}

/// 目的：纯逻辑合成结果（不写进程环境；测试直接断言）。
#[derive(Debug, Clone)]
pub(crate) struct Composed {
    /// 目的：项目 bin 在前、宿主 PATH 在后的生效 PATH。
    pub path: OsString,
    /// 目的：工具需要的环境变量（键值对；按工具声明顺序去重）。
    pub env: Vec<(String, String)>,
    /// 目的：实际生效的项目工具：(id, 版本)。
    pub active: Vec<(String, String)>,
    /// 目的：profile 里声明了但没装上的工具（启动报告从 base_facts 派生；这里留给用例断言）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub missing: Vec<String>,
    /// 目的：路径 / 解析问题（如实报告）。
    pub warnings: Vec<String>,
}

/// 目的：把 active.json 的文本合成为生效 PATH 与环境；不读盘、不改进程环境。
/// 参数：root 是产品根；raw 是 active.json 内容；current_path 是当前 PATH；current_target 是宿主目标。
/// 错误：JSON 非法、目标不符、PATH 拼不出来。
pub(crate) fn compose(
    root: &Path,
    raw: &str,
    current_path: &OsStr,
    current_target: &str,
) -> Result<Composed, String> {
    let active: ActiveFile =
        serde_json::from_str(raw).map_err(|e| format!("active.json 非法：{}", e))?;
    if active.target != current_target {
        return Err(format!(
            "工具链目标不符：active.json 是 {}，当前是 {}",
            active.target, current_target
        ));
    }
    let mut front: Vec<PathBuf> = Vec::new();
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut tools: Vec<(String, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut installed: BTreeSet<String> = BTreeSet::new();
    for tool in &active.tools {
        let mut bins: Vec<PathBuf> = Vec::new();
        for b in &tool.bin {
            let Some(p) = safe_rel(b) else {
                warnings.push(format!("工具 {} 的 bin 路径不安全，已跳过：{}", tool.id, b));
                continue;
            };
            let dir = root.join(p);
            if dir.is_dir() {
                bins.push(dir);
            } else {
                warnings.push(format!("工具 {} 的 bin 目录不在场，已跳过：{}", tool.id, b));
            }
        }
        if bins.is_empty() {
            warnings.push(format!(
                "工具 {} 没有可用的 bin 目录，回退宿主 PATH",
                tool.id
            ));
            continue;
        }
        for (k, v) in &tool.env {
            if !k.trim().is_empty() && !v.trim().is_empty() {
                env.insert(k.clone(), v.clone());
            }
        }
        front.extend(bins);
        installed.insert(tool.id.clone());
        tools.push((tool.id.clone(), tool.version.clone()));
    }
    let missing: Vec<String> = active
        .profile
        .iter()
        .filter(|id| !installed.contains(*id))
        .cloned()
        .collect();
    let mut seen: BTreeSet<OsString> = BTreeSet::new();
    let mut parts: Vec<OsString> = Vec::new();
    for p in front {
        let s = p.into_os_string();
        if seen.insert(s.clone()) {
            parts.push(s);
        }
    }
    for p in std::env::split_paths(current_path) {
        let s = p.into_os_string();
        if seen.insert(s.clone()) {
            parts.push(s);
        }
    }
    let path = std::env::join_paths(parts).map_err(|e| format!("PATH 合成失败：{}", e))?;
    Ok(Composed {
        path,
        env: env.into_iter().collect(),
        active: tools,
        missing,
        warnings,
    })
}

/// 目的：只接受"相对、普通组件"的 bin 路径，拒绝绝对路径与 `..` 逃逸。
fn safe_rel(s: &str) -> Option<PathBuf> {
    let p = Path::new(s);
    if p.as_os_str().is_empty() || p.is_absolute() {
        return None;
    }
    if p.components().all(|c| matches!(c, Component::Normal(_))) {
        Some(p.to_path_buf())
    } else {
        None
    }
}

/// 目的：项目内工具链的只读安装根（构建等系统动作要把它们放行给围栏）。
/// 约束：只认当前目标、相对路径与真实存在的目录；缺失 / 损坏返回空表，不阻断调用方。
pub(crate) fn active_roots(root: &Path) -> Vec<PathBuf> {
    let Some(target) = target() else {
        return Vec::new();
    };
    let file = root.join(".tools").join("toolchains").join("active.json");
    let raw = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let Ok(active) = serde_json::from_str::<ActiveFile>(&raw) else {
        return Vec::new();
    };
    if active.target != target {
        return Vec::new();
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for tool in active.tools {
        for b in tool.bin {
            let Some(p) = safe_rel(&b) else { continue };
            let dir = root.join(p);
            if dir.is_dir() {
                out.push(install_root(&dir));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 目的：bin 目录 → 工具链安装根（<root>/bin 这类布局上溯一层，动态库与 libexec 在 <root> 里）。
fn install_root(bin: &Path) -> PathBuf {
    let leaf = bin
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if leaf == "bin" || leaf == "scripts" {
        if let Some(up) = bin.parent() {
            if up.parent().is_some() {
                return up.to_path_buf();
            }
        }
    }
    bin.to_path_buf()
}

/// 目的：读产品根下的 active.json 并应用到当前进程环境；缺失 / 损坏只报告，不阻断启动。
pub(crate) fn apply(root: &Path) -> Applied {
    let Some(target) = target() else {
        return Applied {
            warnings: vec![format!(
                "当前平台 {} / {} 没有基础工具链条目，使用宿主 PATH",
                std::env::consts::OS,
                std::env::consts::ARCH
            )],
            ..Default::default()
        };
    };
    let file = root.join(".tools").join("toolchains").join("active.json");
    let raw = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) => return Applied::default(),
    };
    let current = std::env::var_os("PATH").unwrap_or_default();
    match compose(root, &raw, &current, target) {
        Ok(c) => {
            std::env::set_var("PATH", &c.path);
            for (k, v) in &c.env {
                std::env::set_var(k, v);
            }
            Applied {
                active: c.active,
                warnings: c.warnings,
            }
        }
        Err(e) => Applied {
            warnings: vec![e],
            ..Default::default()
        },
    }
}

/// 目的：项目内基础 profile 已装上的工具（id, version）——只认当前目标、相对路径与真实存在的 bin。
pub(crate) fn active_tools(root: &Path) -> Vec<(String, String)> {
    let Some(target) = target() else {
        return Vec::new();
    };
    let file = root.join(".tools").join("toolchains").join("active.json");
    let Ok(raw) = std::fs::read_to_string(&file) else {
        return Vec::new();
    };
    let Ok(active) = serde_json::from_str::<ActiveFile>(&raw) else {
        return Vec::new();
    };
    if active.target != target {
        return Vec::new();
    }
    let mut out = Vec::new();
    for tool in active.tools {
        let ok = tool
            .bin
            .iter()
            .any(|b| safe_rel(b).map(|p| root.join(p).is_dir()).unwrap_or(false));
        if ok {
            out.push((tool.id, tool.version));
        }
    }
    out
}

/// 目的：能力名 → 宿主上可能的可执行名（与 toolchains.js 的 profile 一一对应）。
fn host_names(id: &str) -> &'static [&'static str] {
    match id {
        "python" => &["python", "python3"],
        "node" => &["node"],
        "cc" => &["gcc", "g++", "cc", "clang", "clang++"],
        _ => &[],
    }
}

/// 目的：基础 profile 的事实：项目内装了什么 + 缺的那几个在宿主上的回退与围栏可用性。
/// 约束：只读；不写任何权限项。围栏可用性由平台探针回答（Windows 看已有 ACE / 目录属主）。
pub(crate) fn base_facts(root: &Path) -> BaseFacts {
    let active = active_tools(root);
    let installed: BTreeSet<&str> = active.iter().map(|(id, _)| id.as_str()).collect();
    let mut fallback = BTreeMap::new();
    for id in PROFILE {
        if installed.contains(id) {
            continue;
        }
        for name in host_names(id) {
            let Some(exe) = crate::kernel::detail::host_probe::find_exe(name) else {
                continue;
            };
            let dir = crate::kernel::detail::confine::interpreter_install_root(&exe);
            let (fenced, why) = match crate::kernel::detail::confine::interpreter_dir_usable(&dir) {
                Ok(()) => (true, String::new()),
                Err(e) => (false, e),
            };
            fallback.insert(
                id.to_string(),
                HostFallback {
                    path: exe,
                    fenced,
                    why,
                },
            );
            break;
        }
    }
    BaseFacts { active, fallback }
}

/// 目的：每次启动都要打印的基础工具链报告：项目内 / 缺失 / 宿主回退是否可用。
pub(crate) fn report_lines(base: &BaseFacts) -> Vec<String> {
    let installed: BTreeSet<&str> = base.active.iter().map(|(id, _)| id.as_str()).collect();
    let mut lines = Vec::new();
    if !base.active.is_empty() {
        let list: Vec<String> = base
            .active
            .iter()
            .map(|(id, version)| format!("{} {}", id, version))
            .collect();
        lines.push(format!("项目内：{}", list.join("、")));
    }
    let missing: Vec<&str> = PROFILE
        .iter()
        .copied()
        .filter(|id| !installed.contains(id))
        .collect();
    if !missing.is_empty() {
        lines.push(format!(
            "项目内未安装：{}；补全：node setup-runtime.js --yes",
            missing.join("、")
        ));
        for id in missing {
            match base.fallback.get(id) {
                Some(f) if f.fenced => lines.push(format!(
                    "宿主回退 {} → {}（围栏可用）",
                    id,
                    f.path.display()
                )),
                Some(f) => lines.push(format!(
                    "宿主回退 {} → {}（围栏不可授权：{}；工具调用会问是否本次无围栏跑）",
                    id,
                    f.path.display(),
                    f.why
                )),
                None => lines.push(format!("宿主回退 {}：宿主 PATH 也没有", id)),
            }
        }
    }
    lines
}
