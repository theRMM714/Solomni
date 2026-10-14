//! 目的：入口层的依赖补齐机制——依赖清单、下载源解析与"本机缺什么"的状态报告。
//! 管：内嵌 `dependencies.yaml`（依赖清单）与 `links.yaml`（下载源台账）的解析、按本机筛选与状态。
//! 不管：实际下载 / 解包与作业编排（后续增量）、虚拟机档的能力语义（在 capabilities/vm）。
//! 联动：清单与台账的唯一真相在仓库根，产品里**编译期内嵌**；命令行入口是 `--provision`。

use crate::kernel::ports::HostProbe;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 清单与台账在构建期嵌进二进制：分发的二进制不依赖仓库里的文件，读到的仍是同一份真相。
const DEPS_YAML: &str = include_str!("../../dependencies.yaml");
const LINKS_YAML: &str = include_str!("../../links.yaml");

#[derive(Debug, Clone, Deserialize)]
struct DepsFile {
    deps: Vec<Dep>,
}

/// 目的：一条依赖——它是什么、装到哪、怎么判在场、从哪来。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Dep {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub install: String,
    pub probe: String,
    pub source: String,
    #[serde(default)]
    pub size_mb: u64,
    #[serde(default)]
    pub note: String,
}

/// 目的：一条依赖在本机的状态——在不在场、装到哪、从哪个 URL 来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepStatus {
    pub dep: Dep,
    pub present: bool,
    pub path: PathBuf,
    /// 目的：解析出来的下载 URL（`release:` 前缀换成 `release.base`）；解析不了给 None。
    pub url: Option<String>,
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

/// 目的：把 source 解析成可下载的 URL——`release:<资产名>` 走自托管包，其余按 links.yaml 的键。
pub fn source_url(source: &str, links: &BTreeMap<String, String>) -> Result<String, String> {
    if let Some(asset) = source.strip_prefix("release:") {
        let base = links
            .get("release.base")
            .ok_or_else(|| "links.yaml 缺 release.base".to_string())?;
        return Ok(format!("{}/{}", base.trim_end_matches('/'), asset));
    }
    links
        .get(source)
        .cloned()
        .ok_or_else(|| format!("links.yaml 里没有这个键：{}", source))
}

/// 目的：按组件把清单里的相对路径落到产品根下（不用字符串拼分隔符）。
fn under(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        out.push(seg);
    }
    out
}

/// 目的：本机（os/arch）要用的依赖清单（含 `any`）。
pub fn for_host(deps: &[Dep], os: &str, arch: &str) -> Vec<Dep> {
    deps.iter()
        .filter(|d| (d.os == os || d.os == "any") && (d.arch == arch || d.arch == "any"))
        .cloned()
        .collect()
}

/// 目的：本机每一条依赖的状态——probe 指向的路径存在即在位；URL 解析失败如实标 None。
pub fn status(
    root: &Path,
    os: &str,
    arch: &str,
    probe: &dyn HostProbe,
) -> Result<Vec<DepStatus>, String> {
    let links = links()?;
    let mut out = Vec::new();
    for dep in for_host(&deps()?, os, arch) {
        let path = under(root, &dep.probe);
        out.push(DepStatus {
            present: probe.is_file(&path) || probe.is_dir(&path),
            url: source_url(&dep.source, &links).ok(),
            path,
            dep,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::doubles::FixedProbe;

    /// 目的：内嵌清单与台账必须自洽——每条依赖的下载源都解析得成 https。
    #[test]
    fn embedded_manifest_and_links_resolve() {
        let deps = deps().expect("清单可解析");
        assert!(
            deps.iter().any(|d| d.id == "vm.qemu.linux"),
            "Linux QEMU 要在清单里"
        );
        let links = links().expect("台账可解析");
        for d in &deps {
            let url = source_url(&d.source, &links)
                .unwrap_or_else(|e| panic!("{} 的下载源解析不了：{}", d.id, e));
            assert!(
                url.starts_with("https://"),
                "{} 的源不是 https：{}",
                d.id,
                url
            );
        }
        let q = deps.iter().find(|d| d.id == "vm.qemu.linux").unwrap();
        assert!(q.source.starts_with("release:"), "自托管包用 release: 前缀");
    }

    /// 目的：按本机筛出的状态——平台不匹配的不出现，probe 在场才判在位。
    #[test]
    fn host_filter_and_status_report_presence() {
        let root = crate::tests::scratch("provision-status");
        let probe = FixedProbe::default();
        let items = status(&root, "linux", "x86_64", &probe).expect("状态");
        let ids: Vec<&str> = items.iter().map(|s| s.dep.id.as_str()).collect();
        assert!(ids.contains(&"vm.qemu.linux") && ids.contains(&"vm.virtiofsd.linux"));
        assert!(!ids.contains(&"vm.qemu.windows"), "windows 条目不该出现");
        assert!(items.iter().all(|s| !s.present), "什么都没声明 = 全缺");
        assert!(items.iter().all(|s| s.url.is_some()), "每条都要有 URL");

        let want = under(&root, ".vm/qemu/bin/qemu-system-x86_64");
        let mut probe2 = FixedProbe::default();
        probe2.files.push(want);
        let items2 = status(&root, "linux", "x86_64", &probe2).expect("状态");
        let q = items2.iter().find(|s| s.dep.id == "vm.qemu.linux").unwrap();
        assert!(q.present, "probe 在场就该判在位");
        let _ = std::fs::remove_dir_all(&root);
    }
}
