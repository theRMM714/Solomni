//! 运行包契约（package.yaml）与包库事实：一个包 = 一个文件夹 + 一份清单。
//! 包提供的是一个**能力名**（模块的 runtimes 引用的就是它）；模块只声明能力，精确版本由用户定版。
//! 目录遍历机制在适配层（PackageSource 端口）；清单校验、去重、系统路径冲突预检都在本层（纯逻辑，可单测）。
//! 包目录（内容怎么组织）不进本层：本层只认清单事实，装配阶段按 (id, version) 去包里定位。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 能力名/包 id 的合法写法：小写字母或数字开头，其余小写字母、数字、- _ .；长度 1..=64。
pub fn valid_capability(name: &str) -> bool {
    let mut it = name.chars();
    match it.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    if name.len() > 64 {
        return false;
    }
    name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// kind 取值：包整体挂到自己的独立前缀（可重定位、只读、互不干扰）。
pub const KIND_PREFIX: &str = "prefix";
/// kind 取值：包会把文件写进 guest 系统路径（叠层，参与冲突预检）。
pub const KIND_SYSTEM: &str = "system";

fn kind_prefix() -> String {
    KIND_PREFIX.to_string()
}

/// 包清单（package.yaml）——运行包对世界的全部自我介绍。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackageManifest {
    /// 能力名（= 包 id）；同 id 的不同版本可以并存，同 id 同版本只收一份。
    pub id: String,
    /// 精确版本（不含空白与路径分隔符）。
    pub version: String,
    /// prefix（默认）或 system。
    #[serde(default = "kind_prefix")]
    pub kind: String,
    /// kind = prefix 时的独立前缀（guest 内的**书写形式**：相对、/ 分隔）。
    #[serde(default)]
    pub prefix: String,
    /// kind = system 时它写进去的系统路径（同上书写形式）。
    #[serde(default)]
    pub provides_paths: Vec<String>,
    /// 该包自己需要的能力名（传递依赖由 core 求闭包）。
    #[serde(default)]
    pub requires: Vec<String>,
    /// 再分发要随包带上。
    #[serde(default)]
    pub license: Vec<String>,
    #[serde(default)]
    pub note: String,
    /// 包对自身内容的声明（路径 + sha256 + 字节数）：随包库扫描一起解析，当前不参与装配判断，
    /// 也**不用于信任**——用户自造的包没有可信基准。
    #[serde(default)]
    pub files: Vec<FileFingerprint>,
}

/// 一个文件的声明指纹。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileFingerprint {
    /// 包内相对路径（书写形式）。
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
}

/// 包库（依赖文件夹的扫描事实）：合法清单 + 拒收原因（校验，不是挑选）。
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// 按 (id, version) 排序稳定。
    pub packages: Vec<PackageManifest>,
    pub rejected: Vec<String>,
}

impl Library {
    /// 组装包库：逐条校验（清单非法 = 拒收并说明原因）；同一 (id, version) 出现两次只收先出现的。
    /// rejected 由调用方带入（解析失败等适配层事实），与校验拒收合并。
    pub fn build(found: Vec<PackageManifest>, mut rejected: Vec<String>) -> Library {
        let mut packages: Vec<PackageManifest> = Vec::new();
        for manifest in found {
            if let Err(why) = check_manifest(&manifest) {
                rejected.push(format!("{}@{}：{}", manifest.id, manifest.version, why));
                continue;
            }
            if packages.iter().any(|p| p.id == manifest.id && p.version == manifest.version) {
                rejected.push(format!(
                    "{}@{}：同一 id 与版本出现两次（只收先出现的那份）",
                    manifest.id, manifest.version
                ));
                continue;
            }
            packages.push(manifest);
        }
        packages.sort_by(|a, b| (a.id.as_str(), a.version.as_str()).cmp(&(b.id.as_str(), b.version.as_str())));
        Library { packages, rejected }
    }

    /// 提供该能力的所有版本（升序稳定）；空 = 库里没有任何包提供它。
    pub fn versions_of(&self, capability: &str) -> Vec<&PackageManifest> {
        self.packages.iter().filter(|p| p.id == capability).collect()
    }

    /// 定版取包：能力 + 精确版本。
    pub fn pick(&self, capability: &str, version: &str) -> Option<&PackageManifest> {
        self.packages.iter().find(|p| p.id == capability && p.version == version)
    }

    /// 能力名 → 可用版本（呈现用：运行能力报告）。
    pub fn capability_versions(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in &self.packages {
            out.entry(p.id.clone()).or_default().push(p.version.clone());
        }
        out
    }
}

/// 清单校验（纯逻辑；由 Library::build 调用）：非法 = 拒收并说明原因，不纠正。
pub fn check_manifest(m: &PackageManifest) -> Result<(), String> {
    if !valid_capability(&m.id) {
        return Err("id 不合法（只允许小写字母、数字、- _ .，且以字母或数字开头，长度 1..=64）".to_string());
    }
    if m.version.trim().is_empty() || m.version.chars().any(|c| c.is_whitespace() || matches!(c, '/' | '\\')) {
        return Err("version 不能为空，也不能含空白或路径分隔符".to_string());
    }
    match m.kind.as_str() {
        KIND_PREFIX => {
            if !valid_rel_path(&m.prefix) {
                return Err("kind = prefix 时必须给出合法的 prefix（相对、/ 分隔、无空段、无 . 与 ..）".to_string());
            }
            if !m.provides_paths.is_empty() {
                return Err("kind = prefix 的包不该声明 provides_paths（那是 system 类包的事）".to_string());
            }
        }
        KIND_SYSTEM => {
            if m.provides_paths.is_empty() {
                return Err("kind = system 时必须声明 provides_paths（它会写进 guest 系统路径）".to_string());
            }
            for p in &m.provides_paths {
                if !valid_rel_path(p) {
                    return Err(format!("provides_paths 里的路径不合法：{}", p));
                }
            }
        }
        other => return Err(format!("kind 只认 {} 与 {}，收到：{}", KIND_PREFIX, KIND_SYSTEM, other)),
    }
    for r in &m.requires {
        if !valid_capability(r) {
            return Err(format!("requires 里的能力名不合法：{}", r));
        }
        if r == &m.id {
            return Err("requires 不能依赖自己".to_string());
        }
    }
    Ok(())
}

/// 书写形式的相对路径（包前缀与系统路径都用它）：非空、不以 / 开头、无空段、无 . 与 ..、不含反斜杠。
pub fn valid_rel_path(p: &str) -> bool {
    if p.is_empty() || p.starts_with('/') || p.contains('\\') {
        return false;
    }
    p.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// 系统路径冲突预检：两个包会写到同一处（相等或互为前缀）——叠加时互相覆盖，顺序消解不了。
/// 返回（冲突路径, 包A, 包B），按路径排序稳定；只比较被选中的包。
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
                let shorter = if a.len() <= b.len() { a.clone() } else { b.clone() };
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
