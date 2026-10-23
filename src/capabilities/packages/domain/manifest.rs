//! 目的：运行包契约（package.yaml）与包库事实——一个包 = 一个文件夹 + 一份清单。
//! 管：清单校验、去重、系统路径冲突预检与按能力查版本（纯逻辑，可单测）。
//! 不管：目录遍历与 yaml 解析（PackageSource 端口的适配层）、包内容怎么组织（只认清单事实）。
//! 联动：契约见 RUNTIME_SPEC.md；被 domain/plan.rs 与本能力 service 消费。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：kind 取值：包整体挂到自己的独立前缀（可重定位、只读、互不干扰）。
pub const KIND_PREFIX: &str = "prefix";
/// 目的：kind 取值：包会把文件写进 guest 系统路径（叠层，参与冲突预检）。
pub const KIND_SYSTEM: &str = "system";

fn kind_prefix() -> String {
    KIND_PREFIX.to_string()
}

/// 目的：包清单（package.yaml）——运行包对世界的全部自我介绍。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackageManifest {
    /// 目的：能力名（= 包 id）；同 id 的不同版本可以并存，同 id 同版本只收一份。
    pub id: String,
    /// 目的：精确版本（不含空白与路径分隔符）。
    pub version: String,
    /// 目的：prefix（默认）或 system。
    #[serde(default = "kind_prefix")]
    pub kind: String,
    /// 目的：kind = prefix 时的独立前缀（guest 内的**书写形式**：相对、/ 分隔）。
    #[serde(default)]
    pub prefix: String,
    /// 目的：kind = system 时它写进去的系统路径（同上书写形式）。
    #[serde(default)]
    pub provides_paths: Vec<String>,
    /// 目的：该包自己需要的能力名（传递依赖由本能力求闭包）。
    #[serde(default)]
    pub requires: Vec<String>,
    /// 目的：再分发要随包带上。
    #[serde(default)]
    pub license: Vec<String>,
    #[serde(default)]
    pub note: String,
    /// 目的：包对自身内容的声明（路径 + sha256 + 字节数）：随包库扫描一起解析，当前不参与装配判断，
    /// 约束：也**不用于信任**——用户自造的包没有可信基准。
    #[serde(default)]
    pub files: Vec<FileFingerprint>,
    /// 目的：扫描时记下的来源目录（包文件夹的真实路径）；不来自 yaml，装配阶段据此定位包内容。
    #[serde(skip)]
    pub source_dir: std::path::PathBuf,
    /// 目的：扫描时认出的**平台子目录**（键 = 「os-arch」，值 = 那个子目录的真实路径）；空 = 这个包不分平台（整包通用）；不来自 yaml。
    #[serde(skip, default)]
    pub platform_dirs: BTreeMap<String, std::path::PathBuf>,
}

/// 目的：这次装配的**目标**——目标由发起方给，不由包自己声明。
/// 约束：Host = 本机档（只收 `prefix`）；Guest = 虚拟机档（`prefix` 与 `system` 都可以）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 目的：本机档目标（平台 = 宿主 os-arch）。
    Host { platform: String },
    /// 目的：虚拟机档目标（平台 = 那个虚拟根的 os-arch）。
    Guest { platform: String },
}

impl Target {
    /// 目的：宿主目标（平台取当前宿主；认不出平台时只可能是"无平台子目录"的通用包）。
    pub fn host() -> Target {
        Target::Host {
            platform: host_platform().unwrap_or_default().to_string(),
        }
    }

    /// 目的：guest 目标（平台用 pkg 目标的 os-arch 写法，例如 linux-x86_64）。
    pub fn guest(platform: &str) -> Target {
        Target::Guest {
            platform: platform.to_string(),
        }
    }

    /// 目的：目标的平台名（os-arch；可能为空 = 平台未知）。
    pub fn platform(&self) -> &str {
        match self {
            Target::Host { platform } | Target::Guest { platform } => platform,
        }
    }

    /// 目的：是不是本机档目标。
    pub fn is_host(&self) -> bool {
        matches!(self, Target::Host { .. })
    }

    /// 目的：人读的目标（诊断与日志用）。
    pub fn label(&self) -> String {
        match self {
            Target::Host { platform } => format!("本机（{}）", platform_or_unknown(platform)),
            Target::Guest { platform } => format!("guest（{}）", platform_or_unknown(platform)),
        }
    }
}

fn platform_or_unknown(platform: &str) -> &str {
    if platform.is_empty() {
        "平台未知"
    } else {
        platform
    }
}

/// 目的：当前宿主的目标名（与平台子目录同一套 os-arch 写法）；认不出给 None。
pub fn host_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("windows", "aarch64") => Some("windows-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        _ => None,
    }
}

/// 目的：认出的平台子目录：`tier` = None 表示两档通用，Some 表示只服务那一档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformDir<'a> {
    /// 目的：None = 两档通用；Some = 只服务那一档。
    pub tier: Option<crate::kernel::api::Tier>,
    /// 目的：平台名（os-arch）。
    pub platform: &'a str,
}

/// 目的：平台子目录名的写法：`<os>-<arch>`（两档通用）、`host-<os>-<arch>`、`guest-<os>-<arch>`。
/// 约束：不认的子目录当普通内容，不是平台目录。
pub fn platform_dir(name: &str) -> Option<PlatformDir<'_>> {
    let (tier, rest) = if let Some(rest) = name.strip_prefix("host-") {
        (Some(crate::kernel::api::Tier::Host), rest)
    } else if let Some(rest) = name.strip_prefix("guest-") {
        (Some(crate::kernel::api::Tier::Vm), rest)
    } else {
        (None, name)
    };
    let (os, arch) = rest.split_once('-')?;
    if !matches!(os, "linux" | "macos" | "windows") || !matches!(arch, "x86_64" | "aarch64") {
        return None;
    }
    Some(PlatformDir {
        tier,
        platform: rest,
    })
}

/// 目的：这个子目录名是不是平台目录（两档通用或带 host/guest 前缀）。
pub fn is_platform_dir_name(name: &str) -> bool {
    platform_dir(name).is_some()
}

impl PackageManifest {
    /// 目的：这个包在目标下用哪份载荷：带目标的平台目录优先，其次两档通用的 `<os>-<arch>`；
    /// 约束：无平台子目录 = 整包通用；没有匹配载荷 = 该包在该目标不可用（不挂错的那份）。
    pub fn payload_for(&self, target: &Target) -> Option<std::path::PathBuf> {
        if self.platform_dirs.is_empty() {
            return Some(self.source_dir.clone());
        }
        let platform = target.platform();
        if platform.is_empty() {
            return None;
        }
        let key = match target {
            Target::Host { .. } => format!("host-{}", platform),
            Target::Guest { .. } => format!("guest-{}", platform),
        };
        self.platform_dirs
            .get(&key)
            .or_else(|| self.platform_dirs.get(platform))
            .cloned()
    }
}

impl Library {
    /// 目的：按目标筛一遍包库：只留该目标可用的包，并把 `source_dir` 指到选中的载荷。
    /// 约束：本机档只收 `prefix`（`system` 要写系统路径，本机不叠写）；虚拟机档两种都收。
    pub fn for_target(&self, target: &Target) -> Library {
        let mut packages = Vec::new();
        for p in &self.packages {
            if target.is_host() && p.kind != KIND_PREFIX {
                continue;
            }
            let Some(payload) = p.payload_for(target) else {
                continue;
            };
            let mut q = p.clone();
            q.source_dir = payload;
            packages.push(q);
        }
        Library {
            packages,
            rejected: self.rejected.clone(),
        }
    }
}

/// 目的：按 guest 平台筛库（包装 `for_target`，给只需要 guest 的调用方）。
pub fn library_for_guest(lib: &Library, platform: &str) -> Library {
    lib.for_target(&Target::guest(platform))
}

/// 目的：一个文件的声明指纹。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileFingerprint {
    /// 目的：包内相对路径（书写形式）。
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
}

/// 目的：包库（依赖文件夹的扫描事实）：合法清单 + 拒收原因（校验，不是挑选）。
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// 目的：按 (id, version) 排序稳定。
    pub packages: Vec<PackageManifest>,
    pub rejected: Vec<String>,
}

impl Library {
    /// 目的：组装包库：逐条校验（清单非法 = 拒收并说明原因）；同一 (id, version) 出现两次只收先出现的。
    /// 约束：rejected 由调用方带入（解析失败等适配层事实），与校验拒收合并。
    pub fn build(found: Vec<PackageManifest>, mut rejected: Vec<String>) -> Library {
        let mut packages: Vec<PackageManifest> = Vec::new();
        for manifest in found {
            if let Err(why) = check_manifest(&manifest) {
                rejected.push(format!("{}@{}：{}", manifest.id, manifest.version, why));
                continue;
            }
            if packages
                .iter()
                .any(|p| p.id == manifest.id && p.version == manifest.version)
            {
                rejected.push(format!(
                    "{}@{}：同一 id 与版本出现两次（只收先出现的那份）",
                    manifest.id, manifest.version
                ));
                continue;
            }
            packages.push(manifest);
        }
        packages.sort_by(|a, b| {
            (a.id.as_str(), a.version.as_str()).cmp(&(b.id.as_str(), b.version.as_str()))
        });
        Library { packages, rejected }
    }

    /// 目的：提供该能力的所有版本（升序稳定）；空 = 库里没有任何包提供它。
    pub fn versions_of(&self, capability: &str) -> Vec<&PackageManifest> {
        self.packages
            .iter()
            .filter(|p| p.id == capability)
            .collect()
    }

    /// 目的：定版取包：能力 + 精确版本。
    pub fn pick(&self, capability: &str, version: &str) -> Option<&PackageManifest> {
        self.packages
            .iter()
            .find(|p| p.id == capability && p.version == version)
    }

    /// 目的：能力名 → 可用版本（呈现用：运行能力报告）。
    pub fn capability_versions(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in &self.packages {
            out.entry(p.id.clone()).or_default().push(p.version.clone());
        }
        out
    }
}

/// 目的：清单校验（纯逻辑；由 Library::build 调用）：非法 = 拒收并说明原因，不纠正。
pub fn check_manifest(m: &PackageManifest) -> Result<(), String> {
    if !crate::kernel::api::valid_capability(&m.id) {
        return Err(
            "id 不合法（只允许小写字母、数字、- _ .，且以字母或数字开头，长度 1..=64）".to_string(),
        );
    }
    if m.version.trim().is_empty()
        || m.version
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '\\'))
    {
        return Err("version 不能为空，也不能含空白或路径分隔符".to_string());
    }
    match m.kind.as_str() {
        KIND_PREFIX => {
            if !valid_rel_path(&m.prefix) {
                return Err(
                    "kind = prefix 时必须给出合法的 prefix（相对、/ 分隔、无空段、无 . 与 ..）"
                        .to_string(),
                );
            }
            if !m.provides_paths.is_empty() {
                return Err(
                    "kind = prefix 的包不该声明 provides_paths（那是 system 类包的事）".to_string(),
                );
            }
        }
        KIND_SYSTEM => {
            if m.provides_paths.is_empty() {
                return Err(
                    "kind = system 时必须声明 provides_paths（它会写进 guest 系统路径）"
                        .to_string(),
                );
            }
            for p in &m.provides_paths {
                if !valid_rel_path(p) {
                    return Err(format!("provides_paths 里的路径不合法：{}", p));
                }
            }
        }
        other => {
            return Err(format!(
                "kind 只认 {} 与 {}，收到：{}",
                KIND_PREFIX, KIND_SYSTEM, other
            ))
        }
    }
    for r in &m.requires {
        if !crate::kernel::api::valid_capability(r) {
            return Err(format!("requires 里的能力名不合法：{}", r));
        }
        if r == &m.id {
            return Err("requires 不能依赖自己".to_string());
        }
    }
    Ok(())
}

/// 目的：书写形式的相对路径（包前缀与系统路径都用它）：非空、不以 / 开头、无空段、无 . 与 ..、不含反斜杠。
pub fn valid_rel_path(p: &str) -> bool {
    if p.is_empty() || p.starts_with('/') || p.contains('\\') {
        return false;
    }
    p.split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// 目的：系统路径冲突预检：两个包会写到同一处（相等或互为前缀）——叠加时互相覆盖，顺序消解不了。
/// 约束：返回（冲突路径, 包A, 包B），按路径排序稳定；只比较被选中的包。
pub fn conflicts(chosen: &[&PackageManifest]) -> Vec<(String, String, String)> {
    let mut paths: Vec<(String, String)> = Vec::new();
    for p in chosen {
        let label = format!("{}@{}", p.id, p.version);
        if p.kind == KIND_SYSTEM {
            for x in &p.provides_paths {
                paths.push((trim_slashes(x), label.clone()));
            }
        } else {
            paths.push((trim_slashes(&p.prefix), label.clone()));
        }
    }
    let mut out: Vec<(String, String, String)> = Vec::new();
    for i in 0..paths.len() {
        for j in (i + 1)..paths.len() {
            let (a, la) = &paths[i];
            let (b, lb) = &paths[j];
            if la == lb {
                continue; // 同一个包自己的两条路径不算冲突
            }
            if overlaps(a, b) {
                let shorter = if a.len() <= b.len() {
                    a.clone()
                } else {
                    b.clone()
                };
                out.push((shorter, la.clone(), lb.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn trim_slashes(p: &str) -> String {
    p.trim_matches('/').to_string()
}

/// 两段相对路径是否落在同一处（相等，或一方是另一方的前缀目录）。
fn overlaps(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{}/", b)) || b.starts_with(&format!("{}/", a))
}
