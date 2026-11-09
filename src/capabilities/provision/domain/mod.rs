//! 目的：provision 的领域模型与纯规则——清单行、宿主筛选、安装身份与 pin 解析。
//! 管：Dep / ToolchainSpec / GuestProfile 类型，内嵌 dependencies.yaml 与 links.yaml 的解析，
//!   路径落点（under）、宿主筛选（for_host）与安装身份指纹（install_identity）。
//! 不管：网络、磁盘与进程（在 detail）；装什么、按什么顺序装（在 service）。
//! 联动：service 与 detail 都只读这里的类型与纯函数。

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 清单与台账在构建期嵌进二进制：分发的二进制不依赖仓库里的文件，读到的仍是同一份真相。
const DEPS_YAML: &str = include_str!("../../../../dependencies.yaml");
const LINKS_YAML: &str = include_str!("../../../../links.yaml");

#[derive(Debug, Clone, Deserialize)]
struct DepsFile {
    deps: Vec<Dep>,
}

/// 目的：缺省安装方式——解归档。
pub(crate) fn default_kind() -> String {
    "archive".to_string()
}

/// 目的：一条依赖——它是什么、装到哪、怎么判在场、从哪来、怎么装。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Dep {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub install: String,
    pub probe: String,
    pub source: String,
    /// 目的：安装方式：archive（默认，解归档）/ appimage / file（单文件直下）/ guest-rootfs（多步构造）/
    ///   toolchain（基础工具链：装进 .tools/toolchains 并登记 active.json）。
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 目的：归属的虚拟根名（只有 guest 类条目有）；一个虚拟根可以由多条条目共同装。
    #[serde(default)]
    pub guest: Option<String>,
    /// 目的：kind: toolchain 的载荷；缺了就是清单写错（安装时如实拒绝，不猜版本或布局）。
    #[serde(default)]
    pub toolchain: Option<ToolchainSpec>,
    /// 目的：Linux 虚拟根构造用的发行版族（只有 guest-rootfs 有）；没有声明 = 没有构造配方。
    #[serde(default)]
    pub family: Option<String>,
    /// 目的：解包时剥掉归档里的前 N 段目录（便携包常带一层包装目录）；缺省 0。
    #[serde(default)]
    pub strip: usize,
    #[serde(default)]
    pub size_mb: u64,
    /// 目的：这条依赖的归档 / 单文件 sha256（64 位十六进制）——**与它守护的那条连接写在同一条**：
    ///   自托管 release 资产写在这里，链接类源写在 `links.yaml` 的 `<键>.sha256`。
    ///   空 = 没 pin（`latest` 这种会随上游滚动的源只能这样），如实不校验，不假装验过。
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub note: String,
}

/// 目的：基础工具链条目的载荷（kind: toolchain）——装哪个工具、什么版本、哪些 bin 目录进 active.json。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolchainSpec {
    /// 目的：active.json 的工具 id（python / node / cc）。
    pub tool: String,
    /// 目的：安装布局与 active.json 里的版本（Zig 是 Zig 的版本，MinGW 是 GCC 的版本）。
    pub version: String,
    /// 目的：宿主目标名（与 install 路径同一套写法）；安装时与当前宿主比对，不符即拒绝。
    #[serde(default)]
    pub target: Option<String>,
    /// 目的：相对安装目录的 bin 目录（按顺序写进 active.json；启动期把它们前置到 PATH）。
    #[serde(default)]
    pub bins: Vec<String>,
    /// 目的：装完是否写 cc / gcc / g++ / clang 包装——Zig 包用它把 zig cc 装成常规编译器名。
    #[serde(default)]
    pub wrappers: bool,
}

/// 目的：解析内嵌的依赖清单。
pub fn deps() -> Result<Vec<Dep>, String> {
    let parsed: DepsFile =
        yaml_serde::from_str(DEPS_YAML).map_err(|e| format!("dependencies.yaml 非法：{}", e))?;
    Ok(parsed.deps)
}

/// 目的：解析内嵌的下载源台账（扁平的 键: https://…）。
pub fn links() -> Result<BTreeMap<String, String>, String> {
    yaml_serde::from_str(LINKS_YAML).map_err(|e| format!("links.yaml 非法：{}", e))
}

/// 目的：把 source 解析成可下载的 URL——`release:<tag>/<资产名>` 走自托管包，其余按 links.yaml 的键。
pub fn source_url(source: &str, links: &BTreeMap<String, String>) -> Result<String, String> {
    if let Some(asset) = source.strip_prefix("release:") {
        // 优先用调用方给的台账（用例注入本地源）；生产传的就是内嵌那份，结果一致。
        let base = links
            .get("release.base")
            .cloned()
            .or_else(|| link("release.base"))
            .ok_or_else(|| "links.yaml 缺 release.base".to_string())?;
        return Ok(format!("{}/{}", base.trim_end_matches('/'), asset));
    }
    links
        .get(source)
        .cloned()
        .ok_or_else(|| format!("links.yaml 里没有这个键：{}", source))
}

/// 目的：按台账键取 URL——Rust 侧与 Node 侧读同一份台账；这里也是台账消费点（T0 扫 `link("键")`）。
pub(crate) fn link(key: &str) -> Option<String> {
    links().ok().and_then(|m| m.get(key).cloned())
}

/// 目的：按组件把清单里的相对路径落到产品根下（不用字符串拼分隔符）。
pub(crate) fn under(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        out.push(seg);
    }
    out
}
/// 目的：一个虚拟根画像里我们要的几项（id / os / arch / runtimes）。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GuestProfile {
    pub id: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    /// 目的：画像里声明的运行能力（内联列表 `runtimes: [python, node, cc, gcc]`；缺画像 = 空）。
    pub runtimes: Vec<String>,
}
/// 目的：本机（os/arch）要用的依赖清单（含 `any`）。
pub fn for_host(deps: &[Dep], os: &str, arch: &str) -> Vec<Dep> {
    deps.iter()
        .filter(|d| (d.os == os || d.os == "any") && (d.arch == arch || d.arch == "any"))
        .cloned()
        .collect()
}

/// 目的：宿主目标名——与清单 `toolchain.target`、安装布局同一套写法（windows-x86_64 等）。
pub(crate) fn host_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// 目的：这条依赖是不是基础工具链（运行前提：每次启动都要读 active.json）。
pub(crate) fn is_toolchain(dep: &Dep) -> bool {
    dep.kind == "toolchain"
}
/// 目的：安装身份的模式版本——安装布局或判定口径变了就加一，让所有旧安装自动算作过期。
const INSTALL_SCHEMA: u32 = 1;

/// 目的：一条依赖「按当前清单装出来应该是什么」的指纹：清单字段 + pin 的 sha256 + 工具链载荷。
/// 约束：纯函数，不碰网络与磁盘；pin 与校验闸用同一个来源（`dep.sha256` 或 `links.yaml` 的 `<source>.sha256`）。
pub(crate) fn install_identity(dep: &Dep) -> String {
    let pin = pinned_sha256(dep, &links().unwrap_or_default()).unwrap_or_default();
    let tool = dep
        .toolchain
        .as_ref()
        .map(|t| {
            format!(
                "{}|{}|{}|{}|{}",
                t.tool,
                t.version,
                t.target.clone().unwrap_or_default(),
                t.bins.join(","),
                t.wrappers
            )
        })
        .unwrap_or_default();
    let material = format!(
        "schema={}\nid={}\nkind={}\ninstall={}\nprobe={}\nsource={}\npin={}\nguest={}\nfamily={}\nsize={}\ntoolchain={}\n",
        INSTALL_SCHEMA,
        dep.id,
        dep.kind,
        dep.install,
        dep.probe,
        dep.source,
        pin,
        dep.guest.clone().unwrap_or_default(),
        dep.family.clone().unwrap_or_default(),
        dep.size_mb,
        tool
    );
    crate::capabilities::workspace::api::content_hash(material.as_bytes())
}

/// 目的：这条依赖 pin 的 sha256——**校验值与它守护的那条连接写在同一条**：
///   自托管 release 资产在清单条目里（`sha256:`），链接类源在 `links.yaml` 的 `<source>.sha256`。
/// 返回：小写十六进制；没 pin 给 None（`latest` 这种会滚动的源只能这样，如实不校验）。
pub(crate) fn pinned_sha256(dep: &Dep, links: &BTreeMap<String, String>) -> Option<String> {
    let own = dep.sha256.trim();
    if !own.is_empty() {
        return Some(own.to_ascii_lowercase());
    }
    links
        .get(&format!("{}.sha256", dep.source))
        .map(|v| v.trim().to_ascii_lowercase())
}

/// 目的：把两段相对路径拼起来（统一用 `/` 书写，与清单同一套写法）；空段忽略，不产生 `//`。
pub(crate) fn join_rel(base: &str, rel: &str) -> String {
    let mut out = base.trim_end_matches('/').to_string();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(seg);
    }
    out
}
